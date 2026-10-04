//! Layout, chat transcript, and mouse hit areas for the Argos terminal shell.

use std::collections::HashSet;

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::markdown::{self, Piece, Tone};

use super::app::{
    intel_category_short, intel_day_button_label, is_picker_field, unix_now, App, AtlasPage,
    BrainListMode, ButtonId, ChoiceKind, DefaultsRole, FieldId, IntelPage, IntelReconFocus,
    LogLine, ModuleId, Overlay, ProviderPage, Target, INTEL_CATEGORIES,
};
use super::theme;
use argos_osint_core::atlas;
use argos_osint_core::atlas_insights;
use argos_osint_core::intel_recon::{self, ReportMode};
use argos_osint_core::osint;
use argos_osint_core::provider;
use argos_osint_core::recon::{self, Plan};

const TAB_H: u16 = 1;
const PAGE_TAB_H: u16 = 3;
const FIELD_H: u16 = 2;
const ACTION_H: u16 = 3;

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
    #[allow(dead_code)]
    home: Rect,
}

fn composer_height(app: &App) -> u16 {
    if app.module != Some(ModuleId::Recon) || !app.recon_chat {
        return 0;
    }
    let rows = app.input.split('\n').count().max(1) as u16;
    rows.clamp(1, 4)
}

fn chrome(area: Rect, app: &App) -> Chrome {
    let composer_h = composer_height(app);
    let header_h = if app.module.is_none() { 0 } else { TAB_H };
    let rows = split_vertical(
        area,
        [
            Constraint::Length(header_h),
            Constraint::Min(0),
            Constraint::Length(composer_h),
            Constraint::Length(1),
        ],
    );
    let tabs = header_tabs(rows[0]);
    let home = tabs.first().map(|(_, rect)| *rect).unwrap_or_default();
    Chrome {
        header: rows[0],
        body: rows[1],
        composer: rows[2],
        footer: rows[3],
        home,
    }
}

fn header_tabs(area: Rect) -> Vec<(Option<ModuleId>, Rect)> {
    let mut labels = vec!["argos".to_string()];
    labels.extend(
        ModuleId::ALL
            .iter()
            .map(|module| module.title().to_string()),
    );
    let mut x = area.x;
    let mut out = Vec::new();
    for (index, label) in labels.iter().enumerate() {
        let width = (label.len() as u16 + 4).min(area.width.saturating_sub(x - area.x));
        if width < 2 {
            break;
        }
        let module = if index == 0 {
            None
        } else {
            Some(ModuleId::ALL[index - 1])
        };
        out.push((
            module,
            Rect {
                x,
                y: area.y,
                width,
                height: area.height.min(1),
            },
        ));
        x = x.saturating_add(width);
    }
    out
}

fn inset(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

pub(crate) fn contains(rect: Rect, x: u16, y: u16) -> bool {
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
    let send = 8.min(area.width / 5);
    let parts = split_horizontal(area, [Constraint::Min(8), Constraint::Length(send.max(6))]);
    (parts[0], parts[1])
}

struct BrainList {
    actions: Rect,
    query: Rect,
    list: Rect,
    recall: Rect,
}

fn brain_list(area: Rect) -> BrainList {
    let rows = split_vertical(
        area,
        [
            Constraint::Length(ACTION_H),
            Constraint::Length(FIELD_H),
            Constraint::Min(4),
            Constraint::Length(8),
        ],
    );
    BrainList {
        actions: rows[0],
        query: rows[1],
        list: rows[2],
        recall: rows[3],
    }
}

struct BrainForm {
    app: Rect,
    conversation: Rect,
    insight: Rect,
    actions: Rect,
}

fn brain_form(area: Rect) -> BrainForm {
    let rows = split_vertical(
        area,
        [
            Constraint::Length(FIELD_H),
            Constraint::Length(FIELD_H),
            Constraint::Length(FIELD_H),
            Constraint::Length(ACTION_H),
        ],
    );
    BrainForm {
        app: rows[0],
        conversation: rows[1],
        insight: rows[2],
        actions: rows[3],
    }
}

fn provider_areas(area: Rect) -> Vec<Rect> {
    split_vertical(area, [Constraint::Length(PAGE_TAB_H), Constraint::Min(4)])
}

struct BrainPath {
    body: Rect,
}

fn brain_path(area: Rect) -> BrainPath {
    BrainPath { body: area }
}

fn list_room(height: u16) -> usize {
    height.saturating_sub(2) as usize
}

fn in_pane(rect: Rect, x: u16, y: u16) -> bool {
    x > rect.x
        && x + 1 < rect.x.saturating_add(rect.width)
        && y > rect.y
        && y + 1 < rect.y.saturating_add(rect.height)
}

fn model_areas(area: Rect) -> Vec<Rect> {
    split_vertical(
        area,
        [
            Constraint::Length(ACTION_H),
            Constraint::Length(FIELD_H),
            Constraint::Length(FIELD_H),
            Constraint::Length(ACTION_H),
            Constraint::Length(ACTION_H),
            Constraint::Min(0),
        ],
    )
}

fn dashboard_areas(area: Rect) -> (Rect, Rect, Rect) {
    let rows = split_vertical(
        area,
        [
            Constraint::Length(FIELD_H),
            Constraint::Min(0),
            Constraint::Length(ACTION_H),
        ],
    );
    (rows[0], rows[1], rows[2])
}

fn chat_areas(area: Rect) -> (Rect, Rect) {
    let rows = split_vertical(area, [Constraint::Min(0), Constraint::Length(ACTION_H)]);
    (rows[0], rows[1])
}

struct OsintLayout {
    search: Rect,
    list: Rect,
    detail: Rect,
    key: Rect,
    fallback: Rect,
    input: Rect,
    actions: Rect,
}

struct ApiKeySlot {
    field: FieldId,
    fallback: FieldId,
    button: ButtonId,
}

/// Key rows for the selected keyed tool. Each provider shares one primary key and one
/// fallback key, used after the primary account hits a rate or quota limit.
fn api_key_slot(app: &App) -> Option<ApiKeySlot> {
    let id = osint::registry().get(app.tool_sel)?.id;
    let (field, fallback, button) = if id.starts_with("firecrawl_") {
        (
            FieldId::FirecrawlKey,
            FieldId::FirecrawlFallback,
            ButtonId::SaveFirecrawlKey,
        )
    } else if id.starts_with("sociavault_") {
        (
            FieldId::SociaVaultKey,
            FieldId::SociaVaultFallback,
            ButtonId::SaveSociaVaultKey,
        )
    } else if id.starts_with("hunter_") {
        (
            FieldId::HunterKey,
            FieldId::HunterFallback,
            ButtonId::SaveHunterKey,
        )
    } else if id.starts_with("newsapi_") {
        (
            FieldId::NewsApiKey,
            FieldId::NewsApiFallback,
            ButtonId::SaveNewsApiKey,
        )
    } else if id.starts_with("gnews_") {
        (
            FieldId::GnewsKey,
            FieldId::GnewsFallback,
            ButtonId::SaveGnewsKey,
        )
    } else if id.starts_with("newsdata_") {
        (
            FieldId::NewsDataKey,
            FieldId::NewsDataFallback,
            ButtonId::SaveNewsDataKey,
        )
    } else if id.starts_with("currents_") {
        (
            FieldId::CurrentsKey,
            FieldId::CurrentsFallback,
            ButtonId::SaveCurrentsKey,
        )
    } else if id.starts_with("courtlistener_") {
        (
            FieldId::CourtListenerKey,
            FieldId::CourtListenerFallback,
            ButtonId::SaveCourtListenerKey,
        )
    } else {
        return None;
    };
    Some(ApiKeySlot {
        field,
        fallback,
        button,
    })
}

fn osint_areas(area: Rect, with_key: bool) -> OsintLayout {
    let top = split_vertical(area, [Constraint::Length(FIELD_H), Constraint::Min(0)]);
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
                Constraint::Length(FIELD_H),
                Constraint::Length(FIELD_H),
                Constraint::Length(FIELD_H),
                Constraint::Length(ACTION_H),
            ],
        )
    } else {
        split_vertical(
            columns[1],
            [
                Constraint::Min(4),
                Constraint::Length(FIELD_H),
                Constraint::Length(ACTION_H),
            ],
        )
    };
    if with_key {
        OsintLayout {
            search: top[0],
            list: columns[0],
            detail: right[0],
            key: right[1],
            fallback: right[2],
            input: right[3],
            actions: right[4],
        }
    } else {
        OsintLayout {
            search: top[0],
            list: columns[0],
            detail: right[0],
            key: Rect::default(),
            fallback: Rect::default(),
            input: right[1],
            actions: right[2],
        }
    }
}

fn system_areas(area: Rect) -> (Rect, Rect, Rect) {
    let rows = split_vertical(
        area,
        [
            Constraint::Length(6),
            Constraint::Length(ACTION_H),
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
            Constraint::Length(3),
            Constraint::Length(ACTION_H),
            Constraint::Min(0),
        ],
    )
}

fn router_areas(area: Rect) -> Vec<Rect> {
    split_vertical(
        area,
        [
            Constraint::Length(2),
            Constraint::Length(FIELD_H),
            Constraint::Length(ACTION_H),
            Constraint::Length(ACTION_H),
            Constraint::Length(FIELD_H),
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

fn intel_recon_popup_area(area: Rect) -> Rect {
    let width = area.width.clamp(40, 88).min(area.width);
    let height = area.height.saturating_sub(2).clamp(16, 36).min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub(crate) struct HomeRow {
    pub(crate) y: u16,
    pub(crate) target: Option<usize>,
    pub(crate) center: bool,
    pub(crate) kind: HomeKind,
}

pub(crate) enum HomeKind {
    Logo(Line<'static>),
    Heading(&'static str),
    Item { title: String, detail: String },
    Gap,
}

fn home_line(app: &App, x: u16, y: u16) -> Option<usize> {
    let body = chrome(app.screen, app).body;
    if !contains(body, x, y) {
        return None;
    }
    home_rows(body, 0)
        .into_iter()
        .find(|row| row.y == y)
        .and_then(|row| row.target)
}

pub(crate) fn home_rows(area: Rect, errors: usize) -> Vec<HomeRow> {
    let mut rows = if area.width as usize >= logo_width() {
        logo_rows()
    } else {
        vec![center_row(HomeKind::Heading("ARGOS OSINT"))]
    };
    let menu_at = rows.len();
    home_group(
        &mut rows,
        "Applications",
        &[
            ModuleId::Intel,
            ModuleId::Atlas,
            ModuleId::Brain,
            ModuleId::Recon,
        ],
        errors,
    );
    rows.push(gap_row());
    home_group(
        &mut rows,
        "System",
        &[ModuleId::Osint, ModuleId::Providers, ModuleId::System],
        errors,
    );
    let spare = (area.height as usize)
        .saturating_sub(rows.len())
        .min(HOME_LOGO_GAP);
    // Top pad is half the old even split; mid gap stays; leftover spare is bottom room.
    let above = spare / 4;
    let below = spare / 2;
    for _ in 0..below {
        rows.insert(menu_at, gap_row());
    }
    for _ in 0..above {
        rows.insert(0, gap_row());
    }
    let limit = area.height as usize;
    if rows.len() > limit {
        rows.truncate(limit);
    }
    for (index, row) in rows.iter_mut().enumerate() {
        row.y = area.y.saturating_add(index as u16);
    }
    rows
}

fn home_group(rows: &mut Vec<HomeRow>, title: &'static str, modules: &[ModuleId], errors: usize) {
    rows.push(HomeRow {
        y: 0,
        target: None,
        center: false,
        kind: HomeKind::Heading(title),
    });
    for module in modules {
        rows.push(HomeRow {
            y: 0,
            target: Some(module.index()),
            center: false,
            kind: HomeKind::Item {
                title: module.title().to_string(),
                detail: module_detail(*module, errors),
            },
        });
    }
}

fn center_row(kind: HomeKind) -> HomeRow {
    HomeRow {
        y: 0,
        target: None,
        center: true,
        kind,
    }
}

fn gap_row() -> HomeRow {
    HomeRow {
        y: 0,
        target: None,
        center: false,
        kind: HomeKind::Gap,
    }
}

fn module_detail(module: ModuleId, errors: usize) -> String {
    if module == ModuleId::System && errors > 0 {
        format!("{} · {errors} errors", module.blurb())
    } else {
        module.blurb().to_string()
    }
}

fn home_row_width(row: &HomeRow) -> usize {
    match &row.kind {
        HomeKind::Logo(line) => line_width(line),
        HomeKind::Heading(title) => title.chars().count(),
        HomeKind::Item { detail, .. } => 2 + 12 + detail.chars().count(),
        HomeKind::Gap => 0,
    }
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum()
}

/// Cap on spare blank rows around the wordmark (top / mid / unused bottom).
const HOME_LOGO_GAP: usize = 28;

/// ANSI Shadow wordmark. The right-edge and baseline strokes are the shade.
const LOGO: [&str; 6] = [
    " █████╗  ██████╗   ██████╗   ██████╗  ███████╗    ██████╗  ███████╗ ██╗ ███╗   ██╗ ████████╗",
    "██╔══██╗ ██╔══██╗ ██╔════╝  ██╔═══██╗ ██╔════╝   ██╔═══██╗ ██╔════╝ ██║ ████╗  ██║ ╚══██╔══╝",
    "███████║ ██████╔╝ ██║  ███╗ ██║   ██║ ███████╗   ██║   ██║ ███████╗ ██║ ██╔██╗ ██║    ██║",
    "██╔══██║ ██╔══██╗ ██║   ██║ ██║   ██║ ╚════██║   ██║   ██║ ╚════██║ ██║ ██║╚██╗██║    ██║",
    "██║  ██║ ██║  ██║ ╚██████╔╝ ╚██████╔╝ ███████║   ╚██████╔╝ ███████║ ██║ ██║ ╚████║    ██║",
    "╚═╝  ╚═╝ ╚═╝  ╚═╝  ╚═════╝   ╚═════╝  ╚══════╝    ╚═════╝  ╚══════╝ ╚═╝ ╚═╝  ╚═══╝    ╚═╝",
];
const LOGO_SPLIT: [usize; 6] = [50, 49, 49, 49, 49, 50];

fn logo_width() -> usize {
    LOGO.iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0)
}

fn logo_rows() -> Vec<HomeRow> {
    let width = logo_width();
    LOGO.iter()
        .enumerate()
        .map(|(index, line)| {
            let mut padded: String = (*line).to_string();
            while padded.chars().count() < width {
                padded.push(' ');
            }
            let bottom = index + 1 == LOGO.len();
            center_row(HomeKind::Logo(Line::from(paint_logo(
                &padded,
                LOGO_SPLIT[index],
                bottom,
            ))))
        })
        .collect()
}

fn paint_logo(pattern: &str, split: usize, bottom: bool) -> Vec<Span<'static>> {
    pattern
        .chars()
        .enumerate()
        .map(|(index, ch)| {
            let bright = index >= split;
            let (face, shade) = if bright {
                (theme::TEXT, Color::Rgb(78, 108, 120))
            } else {
                (theme::DIM, Color::Rgb(42, 64, 76))
            };
            let shade_stroke = bottom && ch != ' ' || matches!(ch, '╗' | '║' | '╝');
            if ch == ' ' {
                Span::styled(" ", Style::default().bg(theme::BG))
            } else if shade_stroke {
                Span::styled(ch.to_string(), Style::default().fg(shade).bg(theme::BG))
            } else {
                Span::styled(ch.to_string(), Style::default().fg(face).bg(theme::BG))
            }
        })
        .collect()
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
    // One live answer per open thread. It is appended after every tool row so a plan-log
    // refresh or a late call cannot push the text the user is reading off the bottom.
    let mut live: Option<ChatBlock> = None;
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
                }
                if !answered {
                    if let Some((title, body)) = app.live_bubble(&run.thread_id) {
                        live = Some(ChatBlock {
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
    if let Some(block) = live {
        blocks.push(block);
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
    lines.push(
        if matches!(
            plan.directives_mode.as_str(),
            "directives_fallback" | "questions_fallback"
        ) {
            "Directives (fallback set):".to_string()
        } else {
            "Directives:".to_string()
        },
    );
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
            let platform = if binding.qualifier.is_empty() {
                String::new()
            } else {
                format!(" ({})", binding.qualifier)
            };
            if binding.unverified {
                format!(
                    "{} {}{platform} · named in {}, unverified",
                    binding.kind, binding.value, binding.evidence_id
                )
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
    calls
        .iter()
        .find(|call| call.tool_id == step.tool_id && call.inputs == step.arguments)
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
    parts.push(if result.cached {
        "cache".into()
    } else {
        "live".into()
    });
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
        detail.push(format!(
            "Retrieved: {}",
            atlas::friendly_date(&result.retrieved_at)
        ));
    }
    if let Some(error) = result.error.as_deref().filter(|text| !text.is_empty()) {
        detail.push(format!("Error: {error}"));
    }
    let observations = observation_log(&result.observations);
    if !observations.is_empty() && observations != "null" {
        detail.push(observations);
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
    let brief = input_brief(&call.inputs);
    let inputs = if brief.is_empty() {
        String::new()
    } else {
        format!(" · {}", clip_chars(&brief, 72))
    };
    if !app.expanded.contains(&key) {
        return ChatBlock {
            key,
            title: format!("{name} · {}{cache}{inputs}", call.status),
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
    lines.extend(input_lines(&call.inputs));
    if let Some(summary) = result_summary(call) {
        lines.push(format!("Result: {summary}"));
    }
    lines.push("Full result is in the System event log.".into());
    ChatBlock {
        key,
        title: format!("{name} · {}{inputs}", call.status),
        body: lines.join("\n"),
        collapsible: true,
        message_index: None,
        has_memory: false,
    }
}

fn input_brief(inputs: &serde_json::Value) -> String {
    let serde_json::Value::Object(map) = inputs else {
        return String::new();
    };
    map.iter()
        .filter_map(|(key, value)| {
            let shown = match value {
                serde_json::Value::String(text) if !text.is_empty() => text.clone(),
                serde_json::Value::Number(number) => number.to_string(),
                serde_json::Value::Bool(flag) => flag.to_string(),
                _ => return None,
            };
            Some(format!("{key}={shown}"))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn input_lines(inputs: &serde_json::Value) -> Vec<String> {
    let serde_json::Value::Object(map) = inputs else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for (key, value) in map {
        let shown = match value {
            serde_json::Value::String(text) if !text.is_empty() => text.clone(),
            serde_json::Value::Number(number) => number.to_string(),
            serde_json::Value::Bool(flag) => flag.to_string(),
            serde_json::Value::Array(items) if !items.is_empty() => items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<Vec<_>>()
                .join(", "),
            _ => continue,
        };
        if shown.is_empty() {
            continue;
        }
        lines.push(format!("  {key}: {shown}"));
    }
    if lines.is_empty() {
        Vec::new()
    } else {
        let mut out = vec!["Input:".into()];
        out.extend(lines);
        out
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

/// System log text for one observation. Page and extract bodies stay in the
/// synthesis packet; the log keeps the title, URL, and a field count.
fn observation_log(observations: &serde_json::Value) -> String {
    match observations
        .get("evidence_form")
        .and_then(|value| value.as_str())
    {
        Some("page") => page_log(observations),
        Some("extract") => extract_log(observations),
        _ => {
            let rendered = serde_json::to_string_pretty(observations).unwrap_or_default();
            clip_chars(&rendered, 4_000)
        }
    }
}

fn page_label(page: &serde_json::Value) -> String {
    let title = page
        .get("title")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim();
    let url = page
        .get("url")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim();
    match (title.is_empty(), url.is_empty()) {
        (false, false) => format!("{title} · {url}"),
        (false, true) => title.to_string(),
        (true, false) => url.to_string(),
        (true, true) => "page".into(),
    }
}

fn page_log(observations: &serde_json::Value) -> String {
    let Some(pages) = observations.get("pages").and_then(|value| value.as_array()) else {
        return page_label(observations);
    };
    let mut lines = vec![format!(
        "{} page{}",
        pages.len(),
        if pages.len() == 1 { "" } else { "s" }
    )];
    lines.extend(pages.iter().map(page_label));
    lines.join("\n")
}

fn extract_log(observations: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    for key in ["org_name", "domain"] {
        if let Some(text) = observations
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
        {
            parts.push(text.to_string());
        }
    }
    let count = |key: &str| {
        observations
            .get(key)
            .and_then(|value| value.as_array())
            .map(|items| items.len())
            .unwrap_or(0)
    };
    let emails = count("emails");
    let profiles = count("social_profiles");
    let people = count("people");
    if emails > 0 {
        parts.push(format!(
            "{emails} email{}",
            if emails == 1 { "" } else { "s" }
        ));
    }
    if profiles > 0 {
        parts.push(format!(
            "{profiles} profile{}",
            if profiles == 1 { "" } else { "s" }
        ));
    }
    if people > 0 {
        parts.push(if people == 1 {
            "1 person".into()
        } else {
            format!("{people} people")
        });
    }
    if observations
        .get("address")
        .and_then(|value| value.as_str())
        .is_some_and(|text| !text.trim().is_empty())
    {
        parts.push("address".into());
    }
    if let Some(url) = observations
        .get("url")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        parts.push(url.to_string());
    }
    if parts.is_empty() {
        "extract".into()
    } else {
        parts.join(" · ")
    }
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

pub(crate) fn center_line(value: &str, width: usize) -> String {
    center_text(value, width)
}

fn center_text(value: &str, width: usize) -> String {
    let shown = fit(value, width);
    let pad = width.saturating_sub(shown.chars().count()) / 2;
    format!("{:pad$}{shown}", "", pad = pad)
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
    let heading = if label.starts_with("Recon log") {
        Tone::Warn
    } else {
        Tone::Accent
    };
    let mut pieces = vec![
        Piece {
            text: marker.into(),
            tone: heading,
        },
        Piece {
            text: fit(label, label_room),
            tone: heading,
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
                let face = if line.code {
                    RowFace::Code
                } else {
                    RowFace::Plain
                };
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
    inset(chat_areas(body).0)
}

fn chat_view(app: &App) -> (Rect, u16, Vec<ChatRow>) {
    ensure_frame(app);
    let inner = transcript_rect(app);
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
    app.prune_log();
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
    normalize_memories(app);
}

fn normalize_memories(app: &mut App) {
    if app.module != Some(ModuleId::Brain) || app.brain_list_mode != BrainListMode::List {
        return;
    }
    if app.memories.is_empty() {
        app.memory_sel = 0;
        app.scrolls.memories = 0;
        return;
    }
    if app.memory_sel >= app.memories.len() {
        app.memory_sel = app.memories.len() - 1;
    }
    let room = memory_room(app).max(1);
    let max = app.memories.len().saturating_sub(room) as u16;
    if app.scrolls.memories > max {
        app.scrolls.memories = max;
    }
    if matches!(app.focus, Target::Memory(_)) {
        reveal_index(&mut app.scrolls.memories, app.memory_sel, room);
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
            app.move_memory(delta);
        }
        Region::Tools => {
            let max = tool_max(app);
            nudge_list(&mut app.scrolls.tools, delta, max);
        }
        Region::Detail => nudge(&mut app.scrolls.detail, delta * 3, 10_000),
        Region::Recall => nudge(&mut app.scrolls.recall, delta * 3, 10_000),
        Region::Path => nudge(&mut app.scrolls.path, delta, 10_000),
        Region::Summary => nudge(&mut app.scrolls.summary, delta * 3, 10_000),
        Region::Log => {
            let max = log_max(app);
            nudge(&mut app.scrolls.log, delta * 3, max);
        }
        Region::AtlasFeed => shift_atlas_feed(app, delta * 3),
        Region::AtlasOrigins => {
            let max = if app.atlas_page == AtlasPage::Runs && !app.atlas_news {
                atlas_cycle_stats_scroll_max(app)
            } else {
                origins_scroll_max(app)
            };
            nudge(&mut app.scrolls.origins, delta, max);
        }
        Region::AtlasInsights => {
            let max = insights_scroll_max(app);
            nudge(&mut app.scrolls.insights, delta, max);
        }
        Region::AtlasRuns => shift_atlas_runs(app, delta * 3),
        Region::AtlasNews => shift_atlas_articles(app, delta * 3),
        Region::IntelFull => {
            let max = intel_full_scroll_max(app);
            let before = app.scrolls.intel_full;
            nudge(&mut app.scrolls.intel_full, delta * 3, max);
            if app.scrolls.intel_full == before {
                let stack_max = intel_brief_scroll_max(app);
                nudge(&mut app.scrolls.intel_brief, delta * 3, stack_max);
            }
        }
        Region::IntelBrief => {
            let max = intel_brief_scroll_max(app);
            nudge(&mut app.scrolls.intel_brief, delta * 3, max);
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
        Some(ModuleId::Brain)
            if app.brain_list_mode == BrainListMode::List
                && matches!(app.focus, Target::Memory(_)) =>
        {
            let room = memory_room(app).max(1) as i32;
            app.move_memory(direction * room);
        }
        Some(ModuleId::Brain) if app.brain_list_mode == BrainListMode::List => {
            nudge(&mut app.scrolls.recall, direction * 6, 10_000);
        }
        Some(ModuleId::Brain) if app.brain_list_mode == BrainListMode::Graph => {
            nudge(&mut app.scrolls.summary, direction * 4, 10_000);
        }
        Some(ModuleId::Brain) => {}
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
        Some(ModuleId::Atlas) if app.atlas_page == AtlasPage::Runs && app.atlas_news => {
            let room = atlas_news_room(app).max(1) as i32;
            shift_atlas_articles(app, direction * room);
        }
        Some(ModuleId::Atlas) if app.atlas_page == AtlasPage::Runs => {
            if matches!(app.focus, Target::AtlasCycleStats)
                || app
                    .pointer
                    .is_some_and(|(x, y)| matches!(region_at(app, x, y), Region::AtlasOrigins))
            {
                let room = atlas_cycle_stats_room(app).max(1) as i32;
                let max = atlas_cycle_stats_scroll_max(app);
                nudge(&mut app.scrolls.origins, direction * room, max);
                return;
            }
            let room = atlas_runs_room(app).max(1) as i32;
            shift_atlas_runs(app, direction * room);
        }
        Some(ModuleId::Atlas) => {
            if let Some((x, y)) = app.pointer {
                match region_at(app, x, y) {
                    Region::AtlasOrigins => {
                        let room = origins_room(app).max(1) as i32;
                        let max = origins_scroll_max(app);
                        nudge(&mut app.scrolls.origins, direction * room, max);
                        return;
                    }
                    Region::AtlasInsights => {
                        let room = insights_room(app).max(1) as i32;
                        let max = insights_scroll_max(app);
                        nudge(&mut app.scrolls.insights, direction * room, max);
                        return;
                    }
                    _ => {}
                }
            }
            let room = atlas_feed_room(app).max(1) as i32;
            shift_atlas_feed(app, direction * room);
        }
        Some(ModuleId::Intel) if app.intel_page == IntelPage::Briefing => {
            let max = intel_brief_scroll_max(app);
            nudge(&mut app.scrolls.intel_brief, direction * 6, max);
        }
        Some(ModuleId::Intel) => {
            let room = intel_list_room(app).max(1) as i32;
            app.move_intel(direction * room);
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
    AtlasOrigins,
    AtlasInsights,
    AtlasFeed,
    AtlasRuns,
    AtlasNews,
    Path,
    Summary,
    IntelBrief,
    IntelFull,
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
        Some(ModuleId::Brain) if app.brain_list_mode == BrainListMode::Graph => {
            let layout = brain_path(body);
            if contains(layout.body, x, y) {
                let rows = split_vertical(
                    layout.body,
                    [Constraint::Percentage(55), Constraint::Percentage(45)],
                );
                if contains(rows[0], x, y) {
                    Region::Path
                } else {
                    Region::Summary
                }
            } else {
                Region::None
            }
        }
        Some(ModuleId::Brain) if app.brain_list_mode == BrainListMode::List => {
            let layout = brain_list(body);
            if contains(layout.list, x, y) {
                Region::Memories
            } else if contains(layout.recall, x, y) {
                Region::Recall
            } else {
                Region::None
            }
        }
        Some(ModuleId::Brain) => Region::None,
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
        Some(ModuleId::Atlas) => {
            if app.atlas_page == AtlasPage::Runs {
                if app.atlas_news {
                    let (_map, _tabs, list) = atlas_news_areas(body);
                    if contains(list, x, y) {
                        Region::AtlasNews
                    } else {
                        Region::None
                    }
                } else {
                    let (_, _map, stats, cycles) = atlas_runs_areas(body);
                    if contains(cycles, x, y) {
                        Region::AtlasRuns
                    } else if contains(stats, x, y) {
                        Region::AtlasOrigins
                    } else {
                        Region::None
                    }
                }
            } else {
                let (_, table, insights, feed) = atlas_live_areas(body);
                if contains(feed, x, y) {
                    Region::AtlasFeed
                } else if contains(table, x, y) {
                    Region::AtlasOrigins
                } else if contains(insights, x, y) {
                    Region::AtlasInsights
                } else {
                    Region::None
                }
            }
        }
        Some(ModuleId::Intel) if app.intel_page == IntelPage::Briefing => {
            let (_left, center, _right) = intel_briefing_areas(body);
            if !contains(center, x, y) {
                Region::None
            } else if let Some(layout) = intel_center_layout(app, center) {
                if abs_contains(layout.full, x, y) {
                    Region::IntelFull
                } else {
                    Region::IntelBrief
                }
            } else {
                Region::IntelBrief
            }
        }
        Some(ModuleId::Intel) => Region::None,
        None => Region::None,
    }
}

fn thread_room(app: &App) -> usize {
    if app.recon_chat {
        return 1;
    }
    list_room(dashboard_areas(chrome(app.screen, app).body).1.height).max(1)
}

fn memory_room(app: &App) -> usize {
    if app.brain_list_mode != BrainListMode::List {
        return 1;
    }
    (list_room(brain_list(chrome(app.screen, app).body).list.height) / 2).max(1)
}

fn tool_room(app: &App) -> usize {
    list_room(
        osint_areas(chrome(app.screen, app).body, api_key_slot(app).is_some())
            .list
            .height,
    )
    .max(1)
}

fn thread_max(app: &App) -> u16 {
    app.threads.len().saturating_sub(thread_room(app).max(1)) as u16
}

fn tool_max(app: &App) -> u16 {
    visible_tools(app)
        .len()
        .saturating_sub(tool_room(app).max(1)) as u16
}

fn log_pane_width(app: &App) -> usize {
    inset(system_areas(chrome(app.screen, app).body).2)
        .width
        .max(1) as usize
}

fn detail_rows(detail: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for line in detail.lines() {
        let text = format!("  {line}");
        let mut rest = text.as_str();
        if rest.is_empty() {
            rows.push(String::new());
            continue;
        }
        while !rest.is_empty() {
            let end = rest
                .char_indices()
                .nth(width)
                .map(|(index, _)| index)
                .unwrap_or(rest.len());
            rows.push(rest[..end].to_string());
            rest = &rest[end..];
        }
    }
    rows
}

fn open_detail_rows(app: &App, entry: &LogLine) -> usize {
    if app.log_open.contains(&entry.id) && !entry.detail.is_empty() {
        detail_rows(&entry.detail, log_pane_width(app)).len()
    } else {
        0
    }
}

fn log_max(app: &App) -> u16 {
    let room = list_room(system_areas(chrome(app.screen, app).body).2.height) as u16;
    log_line_count(app).saturating_sub(room as usize) as u16
}

fn log_line_count(app: &App) -> usize {
    app.log
        .iter()
        .map(|entry| {
            let extra = open_detail_rows(app, entry);
            1 + extra
        })
        .sum()
}

fn log_entry_start(app: &App, index: usize) -> usize {
    app.log
        .iter()
        .take(index)
        .map(|entry| 1 + open_detail_rows(app, entry))
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
    let room = system_areas(chrome(app.screen, app).body)
        .2
        .height
        .saturating_sub(1)
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

fn atlas_feed_room(app: &App) -> usize {
    inset(atlas_live_areas(chrome(app.screen, app).body).3)
        .height
        .max(1) as usize
}

fn origins_room(app: &App) -> usize {
    inset(atlas_live_areas(chrome(app.screen, app).body).1)
        .height
        .saturating_sub(1)
        .max(1) as usize
}

fn insights_room(app: &App) -> usize {
    inset(atlas_live_areas(chrome(app.screen, app).body).2)
        .height
        .saturating_sub(2)
        .max(1) as usize
}

fn origins_scroll_max(app: &App) -> u16 {
    let rows = if app.atlas_stats.origins.is_empty() {
        1
    } else {
        app.atlas_stats.origins.len()
    };
    rows.saturating_sub(origins_room(app)) as u16
}

fn atlas_cycle_stats_scroll_max(app: &App) -> u16 {
    let stats = selected_cycle_stats(app);
    let rows = stats
        .origins
        .iter()
        .filter(|row| row.articles > 0)
        .count()
        .max(1);
    rows.saturating_sub(atlas_cycle_stats_room(app)) as u16
}

pub fn cycle_stats_under_pointer(app: &App, x: u16, y: u16) -> bool {
    matches!(region_at(app, x, y), Region::AtlasOrigins)
        && app.atlas_page == AtlasPage::Runs
        && !app.atlas_news
}

pub fn shift_cycle_stats(app: &mut App, delta: i32) {
    if delta == 0 {
        return;
    }
    let max = atlas_cycle_stats_scroll_max(app);
    nudge(&mut app.scrolls.origins, delta, max);
}

fn selected_cycle_stats(app: &App) -> atlas::RunStats {
    app.atlas_runs
        .get(app.atlas_run_sel)
        .and_then(|run| serde_json::from_str(&run.stats_json).ok())
        .unwrap_or_default()
}

fn insights_scroll_max(app: &App) -> u16 {
    app.atlas_stats
        .insights
        .rows
        .len()
        .saturating_sub(insights_room(app)) as u16
}

fn atlas_runs_room(app: &App) -> usize {
    inset(atlas_runs_areas(chrome(app.screen, app).body).3)
        .height
        .max(1) as usize
}

fn atlas_cycle_stats_room(app: &App) -> usize {
    inset(atlas_runs_areas(chrome(app.screen, app).body).2)
        .height
        .saturating_sub(3)
        .max(1) as usize
}

fn atlas_news_room(app: &App) -> usize {
    inset(atlas_news_areas(chrome(app.screen, app).body).2)
        .height
        .max(1) as usize
}

pub fn shift_atlas_feed(app: &mut App, delta: i32) {
    if app.atlas_feed.is_empty() || delta == 0 {
        return;
    }
    let last = app.atlas_feed.len() as i32 - 1;
    app.atlas_feed_sel = (app.atlas_feed_sel as i32 + delta).clamp(0, last) as usize;
    app.atlas_feed_follow = app.atlas_feed_sel + 1 == app.atlas_feed.len();
    reveal_atlas_feed(app);
}

pub fn reveal_atlas_feed(app: &mut App) {
    if app.atlas_feed.is_empty() {
        app.scrolls.atlas_feed = 0;
        return;
    }
    if app.atlas_feed_sel >= app.atlas_feed.len() {
        app.atlas_feed_sel = app.atlas_feed.len() - 1;
    }
    let room = atlas_feed_room(app);
    reveal_index(&mut app.scrolls.atlas_feed, app.atlas_feed_sel, room);
    let max = app.atlas_feed.len().saturating_sub(room) as u16;
    if app.scrolls.atlas_feed > max {
        app.scrolls.atlas_feed = max;
    }
}

pub fn shift_atlas_articles(app: &mut App, delta: i32) {
    if app.atlas_articles.is_empty() || delta == 0 {
        return;
    }
    let last = app.atlas_articles.len() as i32 - 1;
    app.atlas_article_sel = (app.atlas_article_sel as i32 + delta).clamp(0, last) as usize;
    let room = atlas_news_room(app);
    reveal_index(&mut app.scrolls.atlas_news, app.atlas_article_sel, room);
    let max = app.atlas_articles.len().saturating_sub(room) as u16;
    if app.scrolls.atlas_news > max {
        app.scrolls.atlas_news = max;
    }
}

pub fn shift_atlas_runs(app: &mut App, delta: i32) {
    if app.atlas_runs.is_empty() || delta == 0 {
        return;
    }
    let last = app.atlas_runs.len() as i32 - 1;
    app.atlas_run_sel = (app.atlas_run_sel as i32 + delta).clamp(0, last) as usize;
    let room = atlas_runs_room(app);
    reveal_index(&mut app.scrolls.atlas_runs, app.atlas_run_sel, room);
    let max = app.atlas_runs.len().saturating_sub(room) as u16;
    if app.scrolls.atlas_runs > max {
        app.scrolls.atlas_runs = max;
    }
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

#[cfg(test)]
pub fn atlas_feed_room_for(app: &App) -> usize {
    atlas_feed_room(app)
}

pub fn atlas_news_room_for(app: &App) -> usize {
    atlas_news_room(app)
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

pub fn atlas_run_card(app: &App) -> bool {
    matches!(&app.overlay, Overlay::Block { title, .. } if title.starts_with("Run "))
}

/// Centered control under the statistics table, inside the card border.
fn run_news_rect(popup: Rect) -> Rect {
    let inner = inset(popup);
    let width = 18.min(inner.width);
    let height = 3.min(inner.height);
    Rect {
        x: inner.x + inner.width.saturating_sub(width) / 2,
        y: inner.y + inner.height.saturating_sub(height),
        width,
        height,
    }
}

fn run_delete_rect(popup: Rect) -> Rect {
    let width = 8.min(popup.width.saturating_sub(8));
    Rect {
        x: popup.x + popup.width.saturating_sub(8 + width),
        y: popup.y,
        width,
        height: 1,
    }
}

pub fn focus_order(app: &App) -> Vec<Target> {
    if app.overlay != Overlay::None {
        if atlas_run_card(app) {
            return vec![
                Target::Button(ButtonId::AtlasNewsFeed),
                Target::Button(ButtonId::AtlasDelete),
                Target::CloseOverlay,
            ];
        }
        if app.overlay == Overlay::IntelRecon {
            let mut order: Vec<Target> = ReportMode::all()
                .into_iter()
                .enumerate()
                .map(|(index, _)| Target::IntelReconTab(index))
                .collect();
            let sections = argos_osint_core::intel_recon::section_plan(app.intel_recon_mode());
            order.extend((0..sections.len()).map(Target::IntelReconSection));
            order.push(Target::Button(ButtonId::IntelReconStart));
            order.push(Target::CloseOverlay);
            return order;
        }
        return vec![Target::CloseOverlay];
    }
    match app.module {
        None => (0..ModuleId::ALL.len()).map(Target::App).collect(),
        Some(ModuleId::Recon) if app.recon_chat => {
            let mut order = vec![Target::Home];
            order.extend((0..ModuleId::ALL.len()).map(Target::App));
            order.push(Target::Transcript);
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
            let mut order = vec![Target::Home];
            order.extend((0..ModuleId::ALL.len()).map(Target::App));
            order.extend([
                Target::Field(FieldId::ReconSearch),
                Target::Button(ButtonId::NewThread),
                Target::Button(ButtonId::DeleteThread),
            ]);
            if !app.threads.is_empty() {
                order.push(Target::Thread(app.thread_sel));
            }
            order
        }
        Some(ModuleId::Brain) => {
            let mut order = vec![Target::Home];
            order.extend((0..ModuleId::ALL.len()).map(Target::App));
            match app.brain_list_mode {
                BrainListMode::Graph => {}
                BrainListMode::Create => {
                    order.extend(
                        [
                            FieldId::BrainApp,
                            FieldId::BrainConversation,
                            FieldId::BrainInsight,
                        ]
                        .map(Target::Field),
                    );
                    order.extend([ButtonId::Add, ButtonId::BrainBack].map(Target::Button));
                }
                _ => {
                    order.push(Target::Button(ButtonId::CreateMemory));
                    order.push(Target::Field(FieldId::BrainQuery));
                    order.extend(
                        [ButtonId::Pin, ButtonId::Delete].map(Target::Button),
                    );
                    if !app.memories.is_empty() {
                        order.push(Target::Memory(app.memory_sel));
                    }
                }
            }
            order
        }
        Some(ModuleId::Osint) => {
            let mut order = vec![Target::Home];
            order.extend((0..ModuleId::ALL.len()).map(Target::App));
            order.push(Target::Field(FieldId::OsintSearch));
            if !visible_tools(app).is_empty() {
                order.push(Target::Tool(app.tool_sel));
            }
            if let Some(slot) = api_key_slot(app) {
                order.push(Target::Field(slot.field));
                order.push(Target::Field(slot.fallback));
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
            order.extend((0..ModuleId::ALL.len()).map(Target::App));
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
        Some(ModuleId::System) => {
            let mut order = vec![Target::Home];
            order.extend((0..ModuleId::ALL.len()).map(Target::App));
            order.extend([
                Target::Button(ButtonId::RefreshHardware),
                Target::Button(ButtonId::ClearLog),
            ]);
            order
        }
        Some(ModuleId::Atlas) => {
            let mut order = vec![Target::Home];
            order.extend((0..ModuleId::ALL.len()).map(Target::App));
            if app.atlas_page == AtlasPage::Runs {
                if app.atlas_news {
                    order.push(Target::Button(ButtonId::AtlasWorld));
                    order.push(Target::Button(ButtonId::AtlasNews));
                    if !app.atlas_articles.is_empty() {
                        order.push(Target::AtlasArticle(app.atlas_article_sel));
                    }
                } else {
                    order.push(Target::Button(ButtonId::AtlasLive));
                    order.push(Target::Button(ButtonId::AtlasDelete));
                    order.push(Target::AtlasCycleStats);
                    if !app.atlas_runs.is_empty() {
                        order.push(Target::AtlasHistory(app.atlas_run_sel));
                    }
                }
            } else {
                order.extend([
                    Target::Button(ButtonId::AtlasRuns),
                    Target::Button(ButtonId::AtlasAuto),
                    Target::Button(ButtonId::AtlasRun),
                ]);
                if !app.atlas_feed.is_empty() {
                    order.push(Target::AtlasFeed(app.atlas_feed_sel));
                }
            }
            order
        }
        Some(ModuleId::Intel) => {
            let mut order = vec![Target::Home];
            order.extend((0..ModuleId::ALL.len()).map(Target::App));
            if app.intel_page == IntelPage::Briefing {
                order.push(Target::Button(ButtonId::IntelBodyRefresh));
                order.push(Target::Button(ButtonId::IntelRecon));
                order.push(Target::Button(ButtonId::IntelJobOpen));
                order.push(Target::Button(ButtonId::IntelJobPause));
                order.push(Target::Button(ButtonId::IntelJobResume));
                order.push(Target::Button(ButtonId::IntelJobCancel));
                order.push(Target::Button(ButtonId::IntelJobRetry));
                order.push(Target::Button(ButtonId::IntelBodyRetry));
            } else {
                order.extend((0..INTEL_CATEGORIES.len()).map(Target::IntelTab));
                order.push(Target::Button(ButtonId::IntelDay));
                order.push(Target::Field(FieldId::IntelSearch));
                if !app.intel_articles.is_empty() {
                    order.push(Target::IntelArticle(app.intel_sel));
                }
            }
            order
        }
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
    if app.overlay == Overlay::Palette {
        let start = app.scrolls.popup as usize;
        let room = inner.height.saturating_sub(1) as usize;
        return app
            .palette_items()
            .into_iter()
            .enumerate()
            .skip(start)
            .take(room)
            .map(|(index, _)| {
                (
                    index,
                    Rect {
                        x: inner.x,
                        y: inner.y + 1 + (index - start) as u16,
                        width: inner.width,
                        height: 1,
                    },
                )
            })
            .collect();
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
        if app.overlay == Overlay::IntelRecon {
            let popup = intel_recon_popup_area(app.screen);
            let close = Rect {
                x: popup.x + popup.width.saturating_sub(8),
                y: popup.y,
                width: 8.min(popup.width),
                height: 1,
            };
            if contains(close, x, y) || !contains(popup, x, y) {
                return Some(Target::CloseOverlay);
            }
            let layout = intel_recon_popup_layout(popup, app);
            for (index, rect) in layout.tabs.into_iter().enumerate() {
                if contains(rect, x, y) {
                    return Some(Target::IntelReconTab(index));
                }
            }
            for (index, rect) in layout.sections.into_iter().enumerate() {
                if contains(rect, x, y) {
                    return Some(Target::IntelReconSection(index));
                }
            }
            if contains(layout.start, x, y) {
                return Some(Target::Button(ButtonId::IntelReconStart));
            }
            return None;
        }
        let popup = popup_area(app.screen);
        let close = Rect {
            x: popup.x + popup.width.saturating_sub(8),
            y: popup.y,
            width: 8.min(popup.width),
            height: 1,
        };
        if atlas_run_card(app) && contains(run_news_rect(popup), x, y) {
            return Some(Target::Button(ButtonId::AtlasNewsFeed));
        }
        if atlas_run_card(app) && contains(run_delete_rect(popup), x, y) {
            return Some(Target::Button(ButtonId::AtlasDelete));
        }
        if contains(close, x, y) || !contains(popup, x, y) {
            return Some(Target::CloseOverlay);
        }
        if matches!(app.overlay, Overlay::Choice(_) | Overlay::Palette) {
            return choice_hits(app)
                .into_iter()
                .find(|(_, rect)| contains(*rect, x, y))
                .map(|(index, _)| Target::Choice(index));
        }
        return None;
    }
    let layout = chrome(app.screen, app);
    if contains(layout.header, x, y) {
        for (module, rect) in header_tabs(layout.header) {
            if contains(rect, x, y) {
                return Some(match module {
                    None => Target::Home,
                    Some(id) => Target::App(
                        ModuleId::ALL
                            .iter()
                            .position(|item| *item == id)
                            .unwrap_or(0),
                    ),
                });
            }
        }
    }
    if app.module.is_none() {
        return home_line(app, x, y).map(Target::App);
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
        Some(ModuleId::Intel) => intel_hit(app, layout.body, x, y),
        Some(ModuleId::Recon) => recon_hit(app, layout.body, x, y),
        Some(ModuleId::Brain) => brain_hit(app, layout.body, x, y),
        Some(ModuleId::Osint) => osint_hit(app, layout.body, x, y),
        Some(ModuleId::Atlas) => atlas_hit(app, layout.body, x, y),
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
    if in_pane(list, x, y) {
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
    if app.brain_list_mode == BrainListMode::Graph {
        let index = super::graph::path_line_at(body, x, y, app.scrolls.path)?;
        return Some(Target::PathLine(index));
    }
    if app.brain_list_mode == BrainListMode::Create {
        let form = brain_form(body);
        if contains(form.app, x, y) {
            return Some(Target::Field(FieldId::BrainApp));
        }
        if contains(form.conversation, x, y) {
            return Some(Target::Field(FieldId::BrainConversation));
        }
        if contains(form.insight, x, y) {
            return Some(Target::Field(FieldId::BrainInsight));
        }
        if contains(form.actions, x, y) {
            let buttons = button_areas(form.actions, 2);
            let ids = [ButtonId::Add, ButtonId::BrainBack];
            return buttons
                .iter()
                .position(|rect| contains(*rect, x, y))
                .map(|index| Target::Button(ids[index]));
        }
        return None;
    }
    let layout = brain_list(body);
    if contains(layout.query, x, y) {
        return Some(Target::Field(FieldId::BrainQuery));
    }
    if contains(layout.actions, x, y) {
        let buttons = button_areas(layout.actions, 3);
        let ids = [
            ButtonId::CreateMemory,
            ButtonId::Pin,
            ButtonId::Delete,
        ];
        return buttons
            .iter()
            .position(|rect| contains(*rect, x, y))
            .map(|index| Target::Button(ids[index]));
    }
    if in_pane(layout.list, x, y) {
        let index = app.scrolls.memories as usize + (y - layout.list.y - 1) as usize / 2;
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
        if contains(layout.fallback, x, y) {
            return Some(Target::Field(slot.fallback));
        }
        if contains(layout.key, x, y) {
            let parts = split_horizontal(layout.key, [Constraint::Min(8), Constraint::Length(16)]);
            return Some(if contains(parts[1], x, y) {
                Target::Button(slot.button)
            } else {
                Target::Field(slot.field)
            });
        }
    }
    if in_pane(list, x, y) {
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
    let (_, actions, log) = system_areas(body);
    if contains(actions, x, y) {
        let areas = button_areas(actions, 2);
        return Some(Target::Button(if contains(areas[0], x, y) {
            ButtonId::RefreshHardware
        } else {
            ButtonId::ClearLog
        }));
    }
    log_index_at(app, log, x, y).map(Target::LogLine)
}

/// The event-log entry under a click. The fold arrow is the first cell of the entry's
/// header row; a click anywhere on that entry folds it.
fn log_index_at(app: &App, log: Rect, x: u16, y: u16) -> Option<usize> {
    let inner = inset(log);
    if app.log.is_empty() || !contains(inner, x, y) {
        return None;
    }
    let row = (y - inner.y) as usize + app.scrolls.log as usize;
    let mut cursor = 0usize;
    for (index, entry) in app.log.iter().enumerate() {
        let extra = if app.log_open.contains(&entry.id) && !entry.detail.is_empty() {
            detail_rows(&entry.detail, inner.width as usize).len()
        } else {
            0
        };
        let height = 1 + extra;
        if row < cursor + height {
            return Some(index);
        }
        cursor += height;
    }
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
        | FieldId::GnewsKey
        | FieldId::NewsDataKey
        | FieldId::CurrentsKey
            if app.module == Some(ModuleId::Osint)
                && api_key_slot(app).is_some_and(|slot| slot.field == field) =>
        {
            let key = osint_areas(layout.body, true).key;
            Some(split_horizontal(key, [Constraint::Min(8), Constraint::Length(16)])[0])
        }
        FieldId::FirecrawlFallback
        | FieldId::HunterFallback
        | FieldId::SociaVaultFallback
        | FieldId::NewsApiFallback
        | FieldId::CourtListenerFallback
        | FieldId::GnewsFallback
        | FieldId::NewsDataFallback
        | FieldId::CurrentsFallback
            if app.module == Some(ModuleId::Osint)
                && api_key_slot(app).is_some_and(|slot| slot.fallback == field) =>
        {
            Some(osint_areas(layout.body, true).fallback)
        }
        FieldId::BrainQuery
            if app.module == Some(ModuleId::Brain)
                && app.brain_list_mode == BrainListMode::List =>
        {
            Some(brain_list(layout.body).query)
        }
        FieldId::BrainApp | FieldId::BrainConversation | FieldId::BrainInsight
            if app.module == Some(ModuleId::Brain)
                && app.brain_list_mode == BrainListMode::Create =>
        {
            let form = brain_form(layout.body);
            match field {
                FieldId::BrainConversation => Some(form.conversation),
                FieldId::BrainInsight => Some(form.insight),
                _ => Some(form.app),
            }
        }
        FieldId::ReconProvider
        | FieldId::ReconModel
        | FieldId::PickerProvider
        | FieldId::PickerModel
        | FieldId::SynthesisProvider
        | FieldId::SynthesisModel
        | FieldId::ClassifierProvider
        | FieldId::ClassifierModel
            if app.module == Some(ModuleId::Providers)
                && app.provider_page == ProviderPage::Defaults =>
        {
            let rows = model_areas(provider_areas(layout.body)[1]);
            Some(match field {
                FieldId::ReconProvider
                | FieldId::PickerProvider
                | FieldId::SynthesisProvider
                | FieldId::ClassifierProvider => rows[1],
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
    let width = area.width.saturating_sub(1) as usize;
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
    let value_area = if field == FieldId::Composer {
        area
    } else {
        field_value_area(area)
    };
    let offset = x.saturating_sub(value_area.x.saturating_add(1)) as usize;
    let value = app.field(field);
    if field != FieldId::Composer {
        return (viewport(app, field, value_area) + offset).min(value.chars().count());
    }
    let width = value_area.width.saturating_sub(1) as usize;
    let (cursor_line, _) = line_col(value, app.cursor);
    let start = value
        .split('\n')
        .take(cursor_line)
        .map(|line| line.chars().count() + 1)
        .sum::<usize>();
    (start + offset.min(width)).min(value.chars().count())
}

fn field_value_area(area: Rect) -> Rect {
    if area.height >= 2 {
        Rect {
            x: area.x,
            y: area.y.saturating_add(1),
            width: area.width,
            height: 1,
        }
    } else {
        area
    }
}

fn draw_field(frame: &mut Frame, app: &App, field: FieldId, label: &str, area: Rect) {
    if area.width < 2 || area.height == 0 {
        return;
    }
    let picker = is_picker_field(field);
    let value = if picker {
        app.field_display(field)
    } else if field == FieldId::Composer {
        app.field(field).to_string()
    } else {
        app.field(field).replace('\n', "⏎")
    };
    let secret = matches!(
        field,
        FieldId::RouterKey
            | FieldId::FirecrawlKey
            | FieldId::FirecrawlFallback
            | FieldId::HunterKey
            | FieldId::HunterFallback
            | FieldId::SociaVaultKey
            | FieldId::SociaVaultFallback
            | FieldId::NewsApiKey
            | FieldId::NewsApiFallback
            | FieldId::CourtListenerKey
            | FieldId::CourtListenerFallback
            | FieldId::GnewsKey
            | FieldId::GnewsFallback
            | FieldId::NewsDataKey
            | FieldId::NewsDataFallback
            | FieldId::CurrentsKey
            | FieldId::CurrentsFallback
    );
    let display = if secret {
        "•".repeat(value.chars().count())
    } else {
        value
    };
    let value_area = field_value_area(area);
    let scroll = if picker {
        0
    } else {
        viewport(app, field, value_area)
    };
    let focused = app.focus == Target::Field(field);
    let gutter = if focused { "▎" } else { " " };
    let width = value_area.width.saturating_sub(1) as usize;
    let empty = if display.is_empty() {
        if picker {
            "Choose".to_string()
        } else if focused {
            String::new()
        } else if field == FieldId::BrainQuery {
            format!("{} memories", app.memories.len())
        } else if field == FieldId::IntelSearch {
            let n = app.intel_articles.len();
            if n == 1 {
                "1 article".into()
            } else {
                format!("{n} articles")
            }
        } else {
            "type to edit".to_string()
        }
    } else {
        String::new()
    };
    let showing_placeholder = !empty.is_empty();
    let center_empty =
        matches!(field, FieldId::BrainQuery | FieldId::IntelSearch) && showing_placeholder;
    let visible: String = if !showing_placeholder {
        display.chars().skip(scroll).take(width).collect()
    } else if center_empty {
        center_text(&empty, width)
    } else {
        empty
    };
    if area.height >= 2 {
        frame.render_widget(
            Paragraph::new(label.trim()).style(if focused {
                theme::accent()
            } else {
                theme::dim()
            }),
            Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1,
            },
        );
    }
    let style = if focused {
        theme::user_message()
    } else if showing_placeholder {
        theme::muted()
    } else {
        theme::text()
    };
    let line = if center_empty {
        format!(" {visible}")
    } else {
        format!("{gutter}{visible}")
    };
    let rows = if field == FieldId::Composer {
        display
            .split('\n')
            .enumerate()
            .map(|(index, row)| {
                let prefix = if focused && index == 0 { "▎" } else { " " };
                format!("{prefix}{}", fit(row, width))
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        line
    };
    frame.render_widget(
        Paragraph::new(rows).style(style),
        value_area_or_composer(area, field),
    );
    if focused && !picker && value_area.width > 1 {
        let (cursor_line, col) = line_col(app.field(field), app.cursor);
        let x = value_area.x
            + 1
            + (col.saturating_sub(scroll) as u16).min(value_area.width.saturating_sub(2));
        let y = if field == FieldId::Composer {
            area.y + cursor_line as u16
        } else {
            value_area.y
        };
        frame.set_cursor_position((x, y.min(area.y + area.height.saturating_sub(1))));
    }
}

fn value_area_or_composer(area: Rect, field: FieldId) -> Rect {
    if field == FieldId::Composer {
        area
    } else {
        field_value_area(area)
    }
}

fn draw_button(frame: &mut Frame, app: &App, button: ButtonId, label: &str, area: Rect) {
    draw_button_state(frame, app, button, label, area, false);
}

fn draw_button_state(
    frame: &mut Frame,
    app: &App,
    button: ButtonId,
    label: &str,
    area: Rect,
    active: bool,
) {
    if area.width < 2 || area.height < 2 {
        return;
    }
    let selected = active || app.focus == Target::Button(button);
    let border = if selected {
        theme::accent()
    } else {
        Style::default().fg(theme::BORDER).bg(theme::BG)
    };
    frame.render_widget(
        Paragraph::new(label)
            .alignment(Alignment::Center)
            .style(if selected {
                theme::selected()
            } else {
                theme::dim()
            })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border)
                    .style(if selected {
                        theme::selected()
                    } else {
                        theme::dim()
                    }),
            ),
        area,
    );
}

fn draw_tabs<T: Copy>(
    frame: &mut Frame,
    area: Rect,
    items: impl IntoIterator<Item = (T, String, bool, bool)>,
) {
    let items: Vec<_> = items.into_iter().collect();
    if items.is_empty() || area.width == 0 {
        return;
    }
    let slots = button_areas(area, items.len());
    for ((_, label, active, focused), rect) in items.into_iter().zip(slots) {
        let style = if active {
            theme::accent().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else if focused {
            theme::selected()
        } else {
            theme::dim()
        };
        frame.render_widget(
            Paragraph::new(label)
                .alignment(Alignment::Center)
                .style(style)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(if active || focused {
                            theme::accent()
                        } else {
                            Style::default().fg(theme::BORDER).bg(theme::BG)
                        })
                        .style(style),
                ),
            rect,
        );
    }
}

fn pane(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER).bg(theme::BG))
        .title(title.to_string())
        .title_style(theme::dim())
        .style(theme::text())
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
        Some(ModuleId::Intel) => draw_intel(frame, app, layout.body),
        Some(ModuleId::Recon) => draw_recon(frame, app, layout.body),
        Some(ModuleId::Brain) => draw_brain(frame, app, layout.body),
        Some(ModuleId::Osint) => draw_osint(frame, app, layout.body),
        Some(ModuleId::Atlas) => draw_atlas(frame, app, layout.body),
        Some(ModuleId::Providers) => draw_providers(frame, app, layout.body),
        Some(ModuleId::System) => draw_system(frame, app, layout.body),
    }
    if composer_height(app) > 0 {
        let (field, send) = composer_parts(layout.composer);
        draw_field(frame, app, FieldId::Composer, "ask", field);
        draw_button(frame, app, ButtonId::Send, "Send", send);
        if app.input.starts_with('/') && app.focus == Target::Field(FieldId::Composer) {
            draw_slash_hint(frame, app, layout.body);
        }
    }
    frame.render_widget(footer_line(app), layout.footer);
    if app.overlay != Overlay::None {
        draw_overlay(frame, app);
    }
}

fn draw_header(frame: &mut Frame, app: &App, layout: &Chrome) {
    let mut spans = Vec::new();
    let mut used = 0u16;
    for (module, rect) in header_tabs(layout.header) {
        used = used.max(rect.x + rect.width - layout.header.x);
        let label = match module {
            None => "argos",
            Some(id) => id.title(),
        };
        let active = match module {
            None => app.module.is_none(),
            Some(id) => app.module == Some(id),
        };
        let focused = match module {
            None => app.focus == Target::Home,
            Some(id) => {
                let index = ModuleId::ALL.iter().position(|item| *item == id);
                index.is_some_and(|slot| app.focus == Target::App(slot))
            }
        };
        let style = if focused {
            theme::selected()
        } else if active {
            theme::accent().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            theme::dim()
        };
        spans.push(Span::styled(format!("│ {label} │"), style));
    }
    let detail = header_detail(app);
    let room = layout.header.width.saturating_sub(used.saturating_add(1)) as usize;
    if room > 4 && !detail.is_empty() {
        spans.push(Span::styled(
            format!(
                " {:>width$}",
                fit(&detail, room.saturating_sub(1)),
                width = room
            ),
            theme::dim(),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(theme::text()),
        layout.header,
    );
}

fn header_detail(app: &App) -> String {
    match app.module {
        None => String::new(),
        Some(ModuleId::Recon) if !app.recon_chat => "investigations".into(),
        Some(ModuleId::Recon) => {
            let title = app
                .threads
                .iter()
                .find(|thread| Some(&thread.id) == app.selected_thread.as_ref())
                .map(|thread| thread.title.as_str())
                .unwrap_or("New investigation");
            title.to_string()
        }
        Some(ModuleId::Brain) => match app.brain_list_mode {
            BrainListMode::List => "memories".into(),
            BrainListMode::Create => "new memory".into(),
            BrainListMode::Graph => {
                if app.brain_graph.is_claim_path()
                    || app
                        .memories
                        .get(app.memory_sel)
                        .is_some_and(|memory| memory.source.app == "atlas")
                {
                    "claim path".into()
                } else {
                    "recon path".into()
                }
            }
        },
        Some(ModuleId::Atlas) if app.atlas_page == AtlasPage::Runs => "history".into(),
        Some(ModuleId::Atlas) => app.atlas_status.clone(),
        Some(ModuleId::Intel) if app.intel_page == IntelPage::Briefing => "briefing focus".into(),
        Some(ModuleId::Intel) => "bulletin board".into(),
        Some(ModuleId::Osint) => "lookup tools".into(),
        Some(ModuleId::Providers) => app.provider_page.title().to_string(),
        Some(ModuleId::System) => {
            let errors = app.error_count();
            if errors == 0 {
                "host".into()
            } else {
                format!("{errors} errors")
            }
        }
    }
}

fn footer_line(app: &App) -> Paragraph<'static> {
    let keys = if matches!(app.overlay, Overlay::Palette) {
        "type to filter · ↑↓ · Enter run · Esc close"
    } else if let Overlay::Choice(_) = app.overlay {
        "↑↓ choose · Enter select · Esc close"
    } else if app.overlay == Overlay::IntelRecon {
        "←→ modes · ↑↓ sections · Space toggle · Enter start · Esc close"
    } else if app.overlay != Overlay::None {
        "Esc close · Ctrl+U/D scroll"
    } else {
        match (app.module, app.focus) {
            (None, _) => "↑↓ open · 1–7 · Ctrl+Tab apps · Ctrl+K · ? help",
            (Some(ModuleId::Intel), _) if app.intel_page == IntelPage::Briefing => {
                "↑↓ scroll · Recon modes · Esc bulletin"
            }
            (Some(ModuleId::Intel), _) => {
                "←→ tabs · ↑↓ articles · Enter brief · / search · Esc home"
            }
            (Some(ModuleId::Recon), Target::Field(FieldId::Composer)) => {
                "Enter send · /commands · Tab transcript · Esc list"
            }
            (
                Some(ModuleId::Recon),
                Target::Transcript | Target::ChatHeader(_) | Target::ChatBody(_),
            ) => "↑↓ select · ←→ fold · Enter toggle · Tab prompt",
            (Some(ModuleId::Recon), _) if !app.recon_chat => {
                "↑↓ open · Ctrl+N new · Ctrl+K · Esc home"
            }
            (Some(ModuleId::Recon), _) => "Tab next · Enter · Ctrl+K · Esc list",
            (Some(ModuleId::Brain), _) if app.brain_list_mode == BrainListMode::Graph => {
                "Esc memories · Ctrl+K"
            }
            (Some(ModuleId::Atlas), _) if app.atlas_news || app.atlas_page == AtlasPage::Live => {
                "Tab next · Enter · Ctrl+K · Esc history"
            }
            (Some(ModuleId::System), _) => "↑↓ log · Enter fold · Ctrl+K · Esc home",
            _ => "Tab next · Ctrl+Tab apps · Enter · Ctrl+K · Esc home",
        }
    };
    let status = status_segments(app);
    let style = if app.status.contains("fail") || app.status.contains("error") {
        theme::error()
    } else {
        theme::dim()
    };
    Paragraph::new(Line::from(vec![
        Span::styled(status, style),
        Span::styled(keys.to_string(), theme::muted()),
    ]))
}

fn status_segments(app: &App) -> String {
    let mut parts = Vec::new();
    if !app.status.is_empty() && app.status != "ready" && app.status != "Home" {
        parts.push(clip_chars(&app.status, 28));
    }
    if app.module == Some(ModuleId::Recon) {
        if !app.recon_model.is_empty() {
            parts.push(clip_chars(&app.recon_model, 22));
        }
        if app.recon_chat {
            if !app.recon_stage.is_empty() {
                parts.push(app.recon_stage.clone());
            }
            if let Some(id) = &app.selected_thread {
                let deadline = app.deadline_label(id);
                if !deadline.is_empty() {
                    parts.push(deadline);
                }
            }
            let done = app
                .calls
                .iter()
                .filter(|call| call.status == "completed" || call.status == "no_results")
                .count();
            if !app.calls.is_empty() {
                parts.push(format!("{done}/{} calls", app.calls.len()));
            }
        }
        let limits = &app.settings.recon_limits;
        parts.push(format!("{}s", limits.turn_seconds));
    } else if app.module == Some(ModuleId::Osint) {
        if let Some(tool) = osint::registry().get(app.tool_sel) {
            parts.push(tool.name.to_string());
        }
    } else if app.module == Some(ModuleId::System) {
        let errors = app.error_count();
        if errors > 0 {
            parts.push(format!("{errors} errors"));
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{}  ·  ", parts.join(" · "))
    }
}

fn draw_slash_hint(frame: &mut Frame, app: &App, body: Rect) {
    let matches = slash_matches(&app.input);
    if matches.is_empty() || body.height < 2 {
        return;
    }
    let height = (matches.len() as u16 + 1)
        .min(body.height.saturating_sub(1))
        .min(8);
    let area = Rect {
        x: body.x,
        y: body.y + body.height.saturating_sub(height),
        width: body.width.clamp(24, 48),
        height,
    };
    let mut lines = vec![Line::from(Span::styled(" commands", theme::card_dim()))];
    for (index, (name, help)) in matches.iter().enumerate() {
        let selected = index == 0;
        lines.push(Line::from(Span::styled(
            format!(" /{name}  {help}"),
            if selected {
                theme::selected()
            } else {
                theme::card_text()
            },
        )));
    }
    cover(frame, area);
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme::card_text())
            .block(theme::card("")),
        area,
    );
}

pub fn slash_matches(input: &str) -> Vec<(&'static str, &'static str)> {
    let typed = input.trim().trim_start_matches('/').to_ascii_lowercase();
    let typed = typed.split_whitespace().next().unwrap_or("");
    SLASH
        .iter()
        .copied()
        .filter(|(name, _)| typed.is_empty() || name.starts_with(typed))
        .collect()
}

const SLASH: &[(&str, &str)] = &[
    ("help", "shortcuts"),
    ("new", "new investigation"),
    ("sessions", "investigation list"),
    ("cancel", "stop the running turn"),
    ("resume", "resume remaining steps"),
    ("insights", "retry insight extraction"),
    ("home", "return home"),
    ("brain", "open Brain"),
    ("osint", "open OSINT tools"),
    ("providers", "open Providers"),
    ("system", "open System"),
    ("palette", "command palette"),
];

fn draw_home(frame: &mut Frame, app: &App, area: Rect) {
    let rows = home_rows(area, app.error_count());
    let column = rows
        .iter()
        .filter(|row| !row.center)
        .map(home_row_width)
        .max()
        .unwrap_or(0) as u16;
    let column = column.min(area.width);
    let left = area.x + area.width.saturating_sub(column) / 2;
    for row in rows {
        if row.y >= area.y.saturating_add(area.height) {
            break;
        }
        let line = home_line_text(&row, app.launcher_sel);
        if line.is_none() {
            continue;
        }
        let rect = if row.center {
            Rect {
                x: area.x,
                y: row.y,
                width: area.width,
                height: 1,
            }
        } else {
            Rect {
                x: left,
                y: row.y,
                width: column,
                height: 1,
            }
        };
        frame.render_widget(
            Paragraph::new(line.unwrap()).alignment(if row.center {
                Alignment::Center
            } else {
                Alignment::Left
            }),
            rect,
        );
    }
}

fn home_line_text(row: &HomeRow, selected: usize) -> Option<Line<'static>> {
    let accent = theme::accent().add_modifier(Modifier::BOLD);
    match &row.kind {
        HomeKind::Gap => None,
        HomeKind::Logo(line) => Some(line.clone()),
        HomeKind::Heading(title) => Some(Line::from(Span::styled((*title).to_string(), accent))),
        HomeKind::Item { title, detail } => {
            let on = row.target == Some(selected);
            let mark = if on { "▸ " } else { "  " };
            let text = format!("{mark}{title:<12}{detail}");
            if on {
                Some(Line::from(Span::styled(text, theme::selected())))
            } else {
                Some(Line::from(vec![
                    Span::styled(
                        format!("{mark}{title:<12}"),
                        theme::text().add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(detail.clone(), theme::dim()),
                ]))
            }
        }
    }
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
    let room = list_room(list.height);
    let width = list.width.saturating_sub(1) as usize;
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
            let when = atlas::friendly_date(&thread.updated_at);
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
    frame.render_widget(List::new(items).block(pane(" investigations ")), list);
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
    frame.render_widget(pane(" transcript "), transcript);
    draw_transcript(frame, app, transcript);
    let run_buttons = button_areas(run_actions, 3);
    draw_button(frame, app, ButtonId::CancelRun, "Cancel", run_buttons[0]);
    draw_button(frame, app, ButtonId::ResumeRun, "Resume", run_buttons[1]);
    draw_button(
        frame,
        app,
        ButtonId::RetryInsights,
        recall_label(app),
        run_buttons[2],
    );
}

fn draw_transcript(frame: &mut Frame, app: &App, _area: Rect) {
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
    super::markdown::style(tone)
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
        .take(list_room(list.height))
        .map(|(index, tool)| {
            let enabled = app.tool_enabled.get(index).copied().unwrap_or(true);
            ListItem::new(format!(
                "{} {} · {}{}",
                if enabled { "●" } else { "○" },
                tool.category,
                tool.name,
                if app.tool_needs_key(tool.id) {
                    " · needs key"
                } else {
                    ""
                }
            ))
            .style(if index == app.tool_sel {
                theme::selected()
            } else {
                theme::text()
            })
        })
        .collect::<Vec<_>>();
    frame.render_widget(List::new(items).block(pane(" tools ")), list);
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
                    result.status,
                    atlas::friendly_date(&result.retrieved_at),
                    result.source_url
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
            .block(pane(" tool "))
            .scroll((app.scrolls.detail, 0))
            .wrap(Wrap { trim: true }),
        detail,
    );
    if let Some(slot) = slot {
        let parts = split_horizontal(layout.key, [Constraint::Min(8), Constraint::Length(16)]);
        draw_field(frame, app, slot.field, " API key ", parts[0]);
        draw_button(frame, app, slot.button, "Save keys", parts[1]);
        draw_field(frame, app, slot.fallback, " Fallback ", layout.fallback);
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
    if app.brain_list_mode == BrainListMode::Graph {
        super::graph::draw(frame, app, area);
        return;
    }
    if app.brain_list_mode == BrainListMode::Create {
        let form = brain_form(area);
        draw_field(frame, app, FieldId::BrainApp, " Source app ", form.app);
        draw_field(
            frame,
            app,
            FieldId::BrainConversation,
            " Conversation ",
            form.conversation,
        );
        draw_field(frame, app, FieldId::BrainInsight, " Insight ", form.insight);
        let actions = button_areas(form.actions, 2);
        draw_button(frame, app, ButtonId::Add, "Save", actions[0]);
        draw_button(frame, app, ButtonId::BrainBack, "Back", actions[1]);
        return;
    }
    let layout = brain_list(area);
    let actions = button_areas(layout.actions, 3);
    draw_button(frame, app, ButtonId::CreateMemory, "Create", actions[0]);
    draw_button(frame, app, ButtonId::Pin, "Pin", actions[1]);
    draw_button(frame, app, ButtonId::Delete, "Delete", actions[2]);
    draw_field(frame, app, FieldId::BrainQuery, " Find ", layout.query);
    let room = list_room(layout.list.height) / 2;
    // Keep each row exactly two lines so room/reveal math matches what is painted.
    let inner_w = layout.list.width.saturating_sub(2) as usize;
    let items = app
        .memories
        .iter()
        .enumerate()
        .skip(app.scrolls.memories as usize)
        .take(room)
        .map(|(index, memory)| {
            let title = fit(
                &format!(
                    "{} [{}] {}",
                    if memory.pinned { "◆" } else { "·" },
                    memory.category,
                    memory.text
                ),
                inner_w,
            );
            let source = fit(
                &format!(
                    "  {} / {}",
                    memory.source.app, memory.source.conversation_id
                ),
                inner_w,
            );
            let selected =
                matches!(app.focus, Target::Memory(_)) && index == app.memory_sel;
            ListItem::new(format!("{title}\n{source}")).style(if selected {
                theme::selected()
            } else {
                theme::text()
            })
        })
        .collect::<Vec<_>>();
    frame.render_widget(List::new(items).block(pane(" memories ")), layout.list);
    let recalled = if matches!(app.focus, Target::Memory(_)) {
        if let Some(insight) = &app.selected_insight {
            memory_anchors_text(insight)
        } else {
            "Find filters saved memory. Select an investigation insight to read its anchors.".into()
        }
    } else {
        "Find filters saved memory. Select an investigation insight to read its anchors.".into()
    };
    frame.render_widget(
        Paragraph::new(recalled)
            .style(theme::text())
            .block(pane(" anchors "))
            .scroll((app.scrolls.recall, 0))
            .wrap(Wrap { trim: true }),
        layout.recall,
    );
}

pub(crate) fn memory_anchors_text(insight: &argos_osint_core::recon::InsightView) -> String {
    let mut lines = vec![format!(
        "{} · {} → {} · {} · {:.0}%",
        insight.entity,
        insight.predicate,
        insight.object_value,
        insight.classification,
        insight.confidence * 100.0
    )];
    let topic = insight.topic.trim();
    if !topic.is_empty() {
        lines.push(format!("topic: {topic}"));
    }
    if insight.sources.is_empty() {
        lines.push("no evidence links".into());
    } else {
        lines.push(format!(
            "{} evidence link{}",
            insight.sources.len(),
            if insight.sources.len() == 1 { "" } else { "s" }
        ));
        for source in &insight.sources {
            lines.push(format!("  {}", source_anchor_label(source)));
        }
    }
    lines.join("\n")
}

fn source_anchor_label(source: &argos_osint_core::recon::InsightSource) -> String {
    if let Some(thread) = source.thread_id.as_deref().filter(|id| !id.is_empty()) {
        if source.deleted_origin {
            format!("deleted origin ({thread})")
        } else {
            format!("thread {thread}")
        }
    } else if let Some(url) = source.source_url.as_deref().filter(|url| !url.trim().is_empty()) {
        url.to_string()
    } else if let Some(run) = source.run_id.as_deref().filter(|id| !id.is_empty()) {
        if source.call_id.is_empty() {
            format!("article run {run}")
        } else {
            format!("article {} · run {run}", source.call_id)
        }
    } else if source.deleted_origin {
        "deleted origin".into()
    } else if !source.call_id.is_empty() {
        format!("source {}", source.call_id)
    } else {
        "source".into()
    }
}

fn draw_providers(frame: &mut Frame, app: &App, area: Rect) {
    let rows = provider_areas(area);
    draw_tabs(
        frame,
        rows[0],
        ProviderPage::ALL.into_iter().map(|page| {
            (
                page,
                page.title().to_string(),
                app.provider_page == page,
                app.focus == Target::ProviderTab(page),
            )
        }),
    );
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
                    .block(pane(" account "))
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
                    .block(pane(" connection "))
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
                    .block(pane(" sign-in "))
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
                    .block(pane(" openrouter "))
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
            let model = app.field(role.model_field());
            let transport = if !model.is_empty()
                && matches!(role, DefaultsRole::ToolPicker | DefaultsRole::Classifier)
            {
                format!(" Transport: {}.", provider::picker_transport(model))
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

fn log_rows(app: &App, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
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
            fit(
                &format!("{marker}{} {} {}", entry.at, entry.level, entry.text),
                width,
            ),
            style,
        )));
        if app.log_open.contains(&entry.id) && !entry.detail.is_empty() {
            for line in detail_rows(&entry.detail, width) {
                rows.push(Line::from(Span::styled(line, theme::text())));
            }
        }
    }
    rows
}

fn atlas_live_areas(area: Rect) -> (Rect, Rect, Rect, Rect) {
    let rows = split_vertical(
        area,
        [
            Constraint::Length(ACTION_H),
            Constraint::Percentage(30),
            Constraint::Percentage(28),
            Constraint::Min(0),
        ],
    );
    (rows[0], rows[1], rows[2], rows[3])
}

fn atlas_runs_areas(area: Rect) -> (Rect, Rect, Rect, Rect) {
    let action_h = ACTION_H.min(area.height);
    let rest = area.height.saturating_sub(action_h);
    let map_h = ((u32::from(rest) * 5 / 8) as u16).min(rest);
    let bottom_h = rest.saturating_sub(map_h);
    let map = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: map_h,
    };
    let actions = Rect {
        x: area.x,
        y: area.y.saturating_add(map_h),
        width: area.width,
        height: action_h,
    };
    let bottom_y = actions.y.saturating_add(action_h);
    let cycle_w = ((u32::from(area.width) * 2 / 5) as u16)
        .max(22)
        .min(area.width.saturating_sub(36));
    let stats_w = area.width.saturating_sub(cycle_w);
    let stats = Rect {
        x: area.x,
        y: bottom_y,
        width: stats_w,
        height: bottom_h,
    };
    let cycles = Rect {
        x: area.x.saturating_add(stats_w),
        y: bottom_y,
        width: cycle_w,
        height: bottom_h,
    };
    (actions, map, stats, cycles)
}

/// Map, then the world/news tabs, then the news list.
fn atlas_news_areas(area: Rect) -> (Rect, Rect, Rect) {
    let tabs_h = 3.min(area.height);
    let remaining = area.height.saturating_sub(tabs_h);
    let map_h = ((u32::from(area.height) * 9 / 16) as u16).min(remaining);
    let list_h = remaining.saturating_sub(map_h);
    let map = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: map_h,
    };
    let tabs = Rect {
        x: area.x,
        y: area.y.saturating_add(map_h),
        width: area.width,
        height: tabs_h,
    };
    let list = Rect {
        x: area.x,
        y: tabs.y.saturating_add(tabs_h),
        width: area.width,
        height: list_h,
    };
    (map, tabs, list)
}

/// Title on the left. Publisher, country code, and category sit on the right.
fn news_feed_line(
    article: &argos_osint_core::store::AtlasArticleRow,
    width: usize,
    selected: bool,
    marked: bool,
) -> Line<'static> {
    let text_style = if selected {
        theme::selected()
    } else {
        theme::text()
    };
    let country = format!(" {} ", article.country.trim().to_ascii_uppercase());
    let (outlet, _) = atlas::publisher_and_author(
        &article.source_name,
        &article.source_domain,
        &article.author,
    );
    let publisher = format!(" {} ", fit(outlet.trim(), 16));
    let category = format!(" {} ", atlas::category_tag(&article.category));
    let mark = if marked { "● " } else { "" };
    let tags_width = mark.chars().count()
        + publisher.chars().count()
        + 1
        + country.chars().count()
        + 1
        + category.chars().count();
    let title = fit(
        &article.title,
        width.saturating_sub(tags_width).saturating_sub(1),
    );
    let gap = width.saturating_sub(title.chars().count() + tags_width);
    let mut spans = Vec::new();
    if marked {
        spans.push(Span::styled(
            "● ",
            ratatui::style::Style::default().fg(ratatui::style::Color::Rgb(255, 214, 64)),
        ));
    }
    spans.extend([
        Span::styled(title, text_style),
        Span::styled(" ".repeat(gap), text_style),
        Span::styled(publisher, theme::accent()),
        Span::styled(" ", text_style),
        Span::styled(country, theme::accent()),
        Span::styled(" ", text_style),
        Span::styled(category, theme::accent()),
    ]);
    Line::from(spans)
}

pub(crate) fn atlas_auto_label(app: &App) -> String {
    match app.atlas_auto_next {
        Some(next) => format!("Auto Run: {}", atlas::friendly_unix(next)),
        None => "Auto Run: Disabled".into(),
    }
}

pub(crate) fn atlas_countdown(next: u64, now: u64) -> String {
    let left = next.saturating_sub(now);
    format!("{}:{:02}:{:02}", left / 3600, (left % 3600) / 60, left % 60)
}

/// History button under the map. Go Live, or the time left until the next automatic run.
pub(crate) fn atlas_history_live_label(app: &App) -> String {
    match app.atlas_auto_next {
        Some(next) => atlas_countdown(next, unix_now()),
        None => "Go Live".into(),
    }
}

fn recall_label(app: &App) -> &'static str {
    let on = app
        .selected_thread
        .as_ref()
        .and_then(|id| app.threads.iter().find(|thread| &thread.id == id))
        .is_some_and(|thread| thread.recall_insights);
    if on { "recall: on" } else { "recall: off" }
}

fn atlas_run_label(app: &App) -> &'static str {
    if app.atlas_pause.is_some() {
        "Pause"
    } else if app.atlas_state == "paused" {
        "Resume"
    } else {
        "Run"
    }
}

pub fn intel_list_room(app: &App) -> usize {
    if app.intel_page != IntelPage::Bulletin {
        return 1;
    }
    list_room(intel_bulletin_areas(chrome(app.screen, app).body).4.height).max(1)
}

fn intel_bulletin_areas(area: Rect) -> (Rect, Rect, Rect, Rect, Rect) {
    let rows = split_vertical(
        area,
        [
            Constraint::Length(PAGE_TAB_H),
            Constraint::Length(ACTION_H),
            Constraint::Percentage(34),
            Constraint::Length(FIELD_H),
            Constraint::Min(4),
        ],
    );
    (rows[0], rows[1], rows[2], rows[3], rows[4])
}

fn intel_briefing_areas(area: Rect) -> (Rect, Rect, Rect) {
    let cols = split_horizontal(
        area,
        [
            Constraint::Percentage(24),
            Constraint::Percentage(48),
            Constraint::Percentage(28),
        ],
    );
    (cols[0], cols[1], cols[2])
}

fn draw_intel(frame: &mut Frame, app: &App, area: Rect) {
    match app.intel_page {
        IntelPage::Bulletin => draw_intel_bulletin(frame, app, area),
        IntelPage::Briefing => draw_intel_briefing(frame, app, area),
    }
}

fn draw_intel_bulletin(frame: &mut Frame, app: &App, area: Rect) {
    let (tabs, title_row, hero, search, list) = intel_bulletin_areas(area);
    let active = INTEL_CATEGORIES
        .iter()
        .position(|id| *id == app.intel_category.as_str())
        .unwrap_or(0);
    draw_tabs(
        frame,
        tabs,
        INTEL_CATEGORIES.iter().enumerate().map(|(index, id)| {
            (
                index,
                intel_category_short(id).to_string(),
                index == active,
                app.focus == Target::IntelTab(index),
            )
        }),
    );
    let day_label = if app.intel_day.is_empty() {
        "No day".into()
    } else {
        intel_day_button_label(&app.intel_day)
    };
    draw_button(frame, app, ButtonId::IntelDay, &day_label, title_row);

    let category_title = format!(" {} ", atlas::category_name(&app.intel_category));
    let story_block = pane(&category_title);
    let story_inner = story_block.inner(hero);
    frame.render_widget(story_block, hero);
    let story_lines = intel_hero_lines(app, story_inner.width as usize);
    frame.render_widget(
        Paragraph::new(story_lines).wrap(Wrap { trim: false }),
        story_inner,
    );

    draw_field(frame, app, FieldId::IntelSearch, "filter", search);

    let list_block = pane(" secondary stories ");
    let list_inner = list_block.inner(list);
    frame.render_widget(list_block, list);
    if app.intel_articles.is_empty() {
        frame.render_widget(
            Paragraph::new(" No classified articles for this day. Run Atlas, or pick another day. ")
                .style(theme::dim()),
            list_inner,
        );
        return;
    }
    let room = list_room(list.height).max(1);
    let start = app.scrolls.intel_list as usize;
    let end = (start + room).min(app.intel_articles.len());
    let items: Vec<ListItem> = app.intel_articles[start..end]
        .iter()
        .enumerate()
        .map(|(offset, article)| {
            let index = start + offset;
            let selected = index == app.intel_sel;
            let style = if selected {
                theme::selected()
            } else {
                theme::text()
            };
            let when = atlas::relative_ago(&article.published_at);
            let width = list_inner.width.max(1) as usize;
            let title = fit(
                &article.title,
                width.saturating_sub(when.chars().count() + 1),
            );
            let gap = width.saturating_sub(title.chars().count() + when.chars().count());
            let when_style = if selected {
                theme::selected()
            } else {
                theme::dim()
            };
            ListItem::new(Line::from(vec![
                Span::styled(title, style),
                Span::styled(" ".repeat(gap), style),
                Span::styled(when, when_style),
            ]))
        })
        .collect();
    frame.render_widget(List::new(items), list_inner);
}

fn intel_hero_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let Some(article) = app.intel_articles.get(app.intel_sel) else {
        return vec![Line::from(Span::styled(
            "Select a story from the list below.",
            theme::dim(),
        ))];
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "● ".to_string(),
                Style::default().fg(super::map::heat_color(article.temperature)),
            ),
            Span::styled(
                fit(&article.title, width.saturating_sub(2)),
                theme::text().add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            format!(
                "{} · {} · {}",
                article.source_name,
                article.country.to_ascii_uppercase(),
                atlas::friendly_date(&article.published_at)
            ),
            theme::dim(),
        )),
        Line::from(""),
    ];
    let body = if article.description.trim().is_empty() {
        "No summary stored for this headline.".to_string()
    } else {
        article.description.clone()
    };
    for chunk in wrap_text(&body, width.max(1)) {
        lines.push(Line::from(Span::styled(chunk, theme::text())));
    }
    lines
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        if paragraph.trim().is_empty() {
            out.push(String::new());
            continue;
        }
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            if line.is_empty() {
                line.push_str(word);
            } else if line.chars().count() + 1 + word.chars().count() <= width {
                line.push(' ');
                line.push_str(word);
            } else {
                out.push(line);
                line = word.to_string();
            }
        }
        if !line.is_empty() {
            out.push(line);
        }
    }
    out
}

fn draw_intel_briefing(frame: &mut Frame, app: &App, area: Rect) {
    let Some(article) = app.intel_articles.get(app.intel_sel) else {
        frame.render_widget(
            Paragraph::new(" No article selected. ").style(theme::dim()),
            area,
        );
        return;
    };
    let (left, center, right) = intel_briefing_areas(area);
    let actors = intel_key_actors(app);
    let locations = intel_locations(app, article);
    let timelines = intel_timelines(app, article);
    draw_intel_side_pane(frame, left, " extracted ", &actors, &locations, &timelines);

    // Side panes stay fixed; only the center stack scrolls.
    frame.render_widget(Block::default().style(theme::text()), center);
    if let Some(layout) = intel_center_layout(app, center) {
        draw_clipped_md_pane(
            frame,
            center,
            layout.brief,
            " brief article ",
            &layout.brief_lines,
            0,
        );
        draw_clipped_button(
            frame,
            app,
            center,
            layout.reload,
            ButtonId::IntelBodyRefresh,
            "Reload",
        );
        if intel_body_loading(app) {
            draw_intel_body_loading(frame, center, layout.full, app);
        } else {
            draw_clipped_md_pane(
                frame,
                center,
                layout.full,
                " full article ",
                &layout.full_lines,
                app.scrolls.intel_full.min(layout.full_scroll_max),
            );
        }
        draw_clipped_divider(frame, center, layout.reports_btn, "Reports");
        draw_clipped_md_pane(
            frame,
            center,
            layout.reports,
            " recon reports ",
            &layout.reports_lines,
            0,
        );
    }

    let right_rows = intel_briefing_right_rows(app, right);
    draw_intel_confidence(frame, app, right_rows[0]);
    draw_intel_tags(frame, article, right_rows[1]);
    super::map::draw_country_mini_map(
        frame,
        right_rows[2],
        &article.country,
        article.temperature,
        "Country",
    );
    draw_button(frame, app, ButtonId::IntelRecon, "Recon", right_rows[3]);
    draw_intel_jobs_pane(frame, app, right_rows[4]);
}

#[derive(Clone, Copy)]
struct AbsRect {
    x: u16,
    y: i32,
    width: u16,
    height: u16,
}

struct IntelCenterLayout {
    brief: AbsRect,
    reload: AbsRect,
    full: AbsRect,
    reports_btn: AbsRect,
    reports: AbsRect,
    brief_lines: Vec<Line<'static>>,
    full_lines: Vec<Line<'static>>,
    reports_lines: Vec<Line<'static>>,
    full_scroll_max: u16,
    stack_scroll_max: u16,
}

fn term_pct_height(app: &App, pct: f32, floor: u16) -> u16 {
    ((f32::from(app.screen.height) * pct).ceil() as u16).max(floor)
}

fn abs_rect(x: u16, y: i32, width: u16, height: u16) -> AbsRect {
    AbsRect {
        x,
        y,
        width,
        height,
    }
}

fn abs_contains(area: AbsRect, x: u16, y: u16) -> bool {
    let y = y as i32;
    x >= area.x
        && x < area.x.saturating_add(area.width)
        && y >= area.y
        && y < area.y + i32::from(area.height)
}

fn intersect_abs(viewport: Rect, area: AbsRect) -> Option<(Rect, u16)> {
    if area.width == 0 || area.height == 0 || viewport.width == 0 || viewport.height == 0 {
        return None;
    }
    let view_y0 = i32::from(viewport.y);
    let view_y1 = view_y0 + i32::from(viewport.height);
    let area_y1 = area.y + i32::from(area.height);
    let y0 = area.y.max(view_y0);
    let y1 = area_y1.min(view_y1);
    if y1 <= y0 {
        return None;
    }
    let x0 = area.x.max(viewport.x);
    let x1 = area
        .x
        .saturating_add(area.width)
        .min(viewport.x.saturating_add(viewport.width));
    if x1 <= x0 {
        return None;
    }
    Some((
        Rect {
            x: x0,
            y: y0 as u16,
            width: x1 - x0,
            height: (y1 - y0) as u16,
        },
        (y0 - area.y) as u16,
    ))
}

fn intel_center_layout(app: &App, viewport: Rect) -> Option<IntelCenterLayout> {
    let article = app.intel_articles.get(app.intel_sel)?;
    if viewport.width < 4 || viewport.height < 4 {
        return None;
    }
    let inner_w = viewport.width.saturating_sub(2).max(1) as usize;
    let min_section = term_pct_height(app, 0.15, 5);
    let full_h = term_pct_height(app, 0.50, 8);

    let brief_lines = intel_brief_preview_lines(article, inner_w);
    let brief_h = (brief_lines.len() as u16)
        .saturating_add(2)
        .max(min_section);

    let full_lines = intel_brief_full_lines(app, inner_w);
    let full_inner = full_h.saturating_sub(2).max(1);
    let full_scroll_max = full_lines.len().saturating_sub(full_inner as usize) as u16;

    let reports_lines = intel_brief_reports_lines(app, inner_w);
    let reports_h = (reports_lines.len() as u16)
        .saturating_add(2)
        .max(min_section);

    let total = brief_h
        .saturating_add(ACTION_H)
        .saturating_add(full_h)
        .saturating_add(ACTION_H)
        .saturating_add(reports_h);
    let stack_scroll_max = total.saturating_sub(viewport.height);
    let scroll = app.scrolls.intel_brief.min(stack_scroll_max) as i32;

    let mut y = i32::from(viewport.y) - scroll;
    let brief = abs_rect(viewport.x, y, viewport.width, brief_h);
    y += i32::from(brief_h);
    let reload = abs_rect(viewport.x, y, viewport.width, ACTION_H);
    y += i32::from(ACTION_H);
    let full = abs_rect(viewport.x, y, viewport.width, full_h);
    y += i32::from(full_h);
    let reports_btn = abs_rect(viewport.x, y, viewport.width, ACTION_H);
    y += i32::from(ACTION_H);
    let reports = abs_rect(viewport.x, y, viewport.width, reports_h);

    Some(IntelCenterLayout {
        brief,
        reload,
        full,
        reports_btn,
        reports,
        brief_lines,
        full_lines,
        reports_lines,
        full_scroll_max,
        stack_scroll_max,
    })
}

pub fn intel_brief_scroll_max(app: &App) -> u16 {
    let body = chrome(app.screen, app).body;
    let (_left, center, _right) = intel_briefing_areas(body);
    intel_center_layout(app, center)
        .map(|layout| layout.stack_scroll_max)
        .unwrap_or(0)
}

fn intel_full_scroll_max(app: &App) -> u16 {
    let body = chrome(app.screen, app).body;
    let (_left, center, _right) = intel_briefing_areas(body);
    intel_center_layout(app, center)
        .map(|layout| layout.full_scroll_max)
        .unwrap_or(0)
}

fn draw_clipped_md_pane(
    frame: &mut Frame,
    viewport: Rect,
    area: AbsRect,
    title: &str,
    lines: &[Line<'static>],
    scroll: u16,
) {
    let Some((vis, top_clip)) = intersect_abs(viewport, area) else {
        return;
    };
    frame.render_widget(Block::default().style(theme::text()), vis);
    stroke_clipped_box(frame, viewport, area, title);

    let inner = AbsRect {
        x: area.x.saturating_add(1),
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let Some((inner_vis, inner_top)) = intersect_abs(viewport, inner) else {
        return;
    };
    let _ = top_clip;
    let skip = scroll as usize + inner_top as usize;
    let visible: Vec<Line> = lines
        .iter()
        .skip(skip)
        .take(inner_vis.height as usize)
        .cloned()
        .collect();
    frame.render_widget(Paragraph::new(visible), inner_vis);
}

fn stroke_clipped_box(frame: &mut Frame, viewport: Rect, area: AbsRect, title: &str) {
    let Some((vis, top_clip)) = intersect_abs(viewport, area) else {
        return;
    };
    if area.width < 2 || area.height < 2 {
        return;
    }
    let border = Style::default().fg(theme::BORDER).bg(theme::BG);
    let title_style = theme::dim();
    let buf = frame.buffer_mut();
    let left = area.x;
    let right = area.x.saturating_add(area.width.saturating_sub(1));
    let top = area.y;
    let bottom = area.y + i32::from(area.height) - 1;

    for row in vis.y..vis.y.saturating_add(vis.height) {
        if left >= vis.x && left < vis.x.saturating_add(vis.width) {
            buf[(left, row)].set_char('│').set_style(border);
        }
        if right >= vis.x && right < vis.x.saturating_add(vis.width) {
            buf[(right, row)].set_char('│').set_style(border);
        }
    }

    if top_clip == 0 {
        let row = top as u16;
        if row >= viewport.y && row < viewport.y.saturating_add(viewport.height) {
            for x in vis.x..vis.x.saturating_add(vis.width) {
                let ch = if x == left {
                    '┌'
                } else if x == right {
                    '┐'
                } else {
                    '─'
                };
                buf[(x, row)].set_char(ch).set_style(border);
            }
            let label = title.chars().take(area.width.saturating_sub(2) as usize);
            let mut x = area.x.saturating_add(1);
            for ch in label {
                if x >= right {
                    break;
                }
                if x >= vis.x && x < vis.x.saturating_add(vis.width) {
                    buf[(x, row)]
                        .set_char(ch)
                        .set_style(title_style);
                }
                x = x.saturating_add(1);
            }
        }
    }

    let bottom_visible = top_clip.saturating_add(vis.height) >= area.height;
    if bottom_visible {
        let row = bottom as u16;
        if bottom >= 0
            && row >= viewport.y
            && row < viewport.y.saturating_add(viewport.height)
        {
            for x in vis.x..vis.x.saturating_add(vis.width) {
                let ch = if x == left {
                    '└'
                } else if x == right {
                    '┘'
                } else {
                    '─'
                };
                buf[(x, row)].set_char(ch).set_style(border);
            }
        }
    }
}

fn draw_clipped_button(
    frame: &mut Frame,
    app: &App,
    viewport: Rect,
    area: AbsRect,
    button: ButtonId,
    label: &str,
) {
    let Some((vis, top_clip)) = intersect_abs(viewport, area) else {
        return;
    };
    if top_clip > 0 || vis.height < area.height || vis.width < area.width {
        // Only activate when the full control is on-screen.
        if vis.height < 2 || vis.width < 4 {
            return;
        }
        frame.render_widget(
            Paragraph::new(label)
                .alignment(Alignment::Center)
                .style(theme::dim())
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(theme::BORDER).bg(theme::BG))
                        .style(theme::dim()),
                ),
            vis,
        );
        return;
    }
    draw_button(frame, app, button, label, vis);
}

fn draw_clipped_divider(frame: &mut Frame, viewport: Rect, area: AbsRect, label: &str) {
    let Some((vis, _)) = intersect_abs(viewport, area) else {
        return;
    };
    if vis.height < 2 || vis.width < 2 {
        return;
    }
    frame.render_widget(
        Paragraph::new(label)
            .alignment(Alignment::Center)
            .style(theme::dim())
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme::BORDER).bg(theme::BG))
                    .style(theme::dim()),
            ),
        vis,
    );
}

fn intel_briefing_right_rows(app: &App, right: Rect) -> Vec<Rect> {
    // Map must be at least 30% of total terminal height.
    let map_floor = term_pct_height(app, 0.30, 8);
    let reserved = 8u16.saturating_add(3).saturating_add(ACTION_H).saturating_add(4);
    let map_h = map_floor.min(right.height.saturating_sub(reserved)).max(8);
    split_vertical(
        right,
        [
            Constraint::Length(8),
            Constraint::Length(3),
            Constraint::Length(map_h),
            Constraint::Length(ACTION_H),
            Constraint::Min(4),
        ],
    )
}

fn draw_intel_side_pane(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    actors: &[String],
    locations: &[String],
    timelines: &[String],
) {
    let block = pane(title);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "KEY ACTORS",
        theme::accent().add_modifier(Modifier::BOLD),
    )));
    if actors.is_empty() {
        lines.push(Line::from(Span::styled("· none extracted", theme::dim())));
    } else {
        for item in actors {
            lines.push(Line::from(Span::styled(format!("· {item}"), theme::text())));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "EVENT LOCATIONS",
        theme::accent().add_modifier(Modifier::BOLD),
    )));
    if locations.is_empty() {
        lines.push(Line::from(Span::styled("· unknown", theme::dim())));
    } else {
        for item in locations {
            lines.push(Line::from(Span::styled(format!("· {item}"), theme::text())));
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "EVENT DATES",
        theme::accent().add_modifier(Modifier::BOLD),
    )));
    if timelines.is_empty() {
        lines.push(Line::from(Span::styled("· unknown", theme::dim())));
    } else {
        for item in timelines {
            lines.push(Line::from(Span::styled(format!("· {item}"), theme::text())));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_intel_jobs_pane(frame: &mut Frame, app: &App, area: Rect) {
    let block = pane(" recon jobs ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if app.intel_jobs.is_empty() {
        frame.render_widget(
            Paragraph::new("No Recon jobs yet.")
                .style(theme::dim())
                .wrap(Wrap { trim: false }),
            inner,
        );
        return;
    }
    let mut lines = Vec::new();
    let start = app.scrolls.intel_jobs as usize;
    for (offset, job) in app.intel_jobs.iter().enumerate().skip(start) {
        let selected = offset == app.intel_job_sel;
        let style = if selected {
            theme::selected()
        } else {
            theme::text()
        };
        let mode = argos_osint_core::intel_recon::ReportMode::parse(&job.mode)
            .map(|m| m.title())
            .unwrap_or(job.mode.as_str());
        lines.push(Line::from(Span::styled(
            format!("{mode} r{} · {}", job.revision, job.state),
            style.add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "  {} · sec {}/{} · el {}/{} · tools {}/{}",
                job.stage,
                job.sections_done,
                job.sections_total,
                job.elements_done,
                job.elements_total,
                job.tool_calls_done,
                job.tool_calls_allowance
            ),
            theme::dim(),
        )));
        if !job.current_tool.is_empty() || !job.warning.is_empty() {
            lines.push(Line::from(Span::styled(
                format!(
                    "  {}{}",
                    job.current_tool,
                    if job.warning.is_empty() {
                        String::new()
                    } else {
                        format!(" · {}", job.warning)
                    }
                ),
                theme::dim(),
            )));
        }
        if lines.len() >= inner.height as usize {
            break;
        }
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_intel_confidence(frame: &mut Frame, app: &App, area: Rect) {
    let block = pane(" confidence ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let eval = intel_source_evaluation(app);
    let (_high, mean) = intel_confidence_scores(app);
    let grade = format!("{}{}", eval.reliability_letter, eval.credibility_digit);
    let score = (eval.reliability_meter + eval.credibility_meter) * 0.5;
    let color = grade_score_color(score);
    let art = grade_ascii_lines(&grade);
    let art_w = art
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0) as u16;
    let cols = split_horizontal(
        inner,
        [
            Constraint::Length(art_w.saturating_add(1).max(12)),
            Constraint::Min(12),
        ],
    );
    let art_lines: Vec<Line> = art
        .into_iter()
        .map(|row| {
            Line::from(Span::styled(
                row,
                Style::default().fg(color).bg(theme::BG).add_modifier(Modifier::BOLD),
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(art_lines), cols[0]);

    let text_w = cols[1].width.max(1) as usize;
    let mut explain = Vec::new();
    explain.push(Line::from(Span::styled(
        fit(
            &format!(
                "Letter {} — {}",
                eval.reliability_letter, eval.reliability_label
            ),
            text_w,
        ),
        theme::text(),
    )));
    explain.push(Line::from(Span::styled(
        fit(
            &format!(
                "Digit {} — {}",
                eval.credibility_digit, eval.credibility_label
            ),
            text_w,
        ),
        theme::text(),
    )));
    explain.push(Line::from(""));
    explain.push(Line::from(Span::styled(
        fit(&format!("Brief rating value: {mean:.2}"), text_w),
        theme::accent().add_modifier(Modifier::BOLD),
    )));
    for chunk in wrap_text(
        "Average confidence across every claim made in the article.",
        text_w,
    ) {
        explain.push(Line::from(Span::styled(chunk, theme::dim())));
    }
    if !eval.footnote.is_empty() {
        explain.push(Line::from(Span::styled(
            fit(&eval.footnote, text_w),
            theme::dim(),
        )));
    }
    frame.render_widget(Paragraph::new(explain), cols[1]);
}

/// Bright green (best) → bright red (worst) by combined Admiralty meter.
fn grade_score_color(score: f64) -> Color {
    let t = score.clamp(0.0, 1.0);
    let r = (255.0 * (1.0 - t) + 20.0 * t).round() as u8;
    let g = (40.0 * (1.0 - t) + 255.0 * t).round() as u8;
    let b = (40.0 * (1.0 - t) + 90.0 * t).round() as u8;
    Color::Rgb(r, g, b)
}

/// ANSI Shadow glyphs (same family as the ARGOS home wordmark), A–F / 0–9.
fn grade_ascii_lines(grade: &str) -> Vec<String> {
    let glyphs: Vec<[&str; 6]> = grade.chars().filter_map(ansi_shadow_glyph).collect();
    if glyphs.is_empty() {
        return vec!["?".into(); 6];
    }
    (0..6)
        .map(|row| {
            glyphs
                .iter()
                .map(|g| g[row])
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

fn ansi_shadow_glyph(ch: char) -> Option<[&'static str; 6]> {
    Some(match ch.to_ascii_uppercase() {
        'A' => [
            " █████╗ ",
            "██╔══██╗",
            "███████║",
            "██╔══██║",
            "██║  ██║",
            "╚═╝  ╚═╝",
        ],
        'B' => [
            "██████╗ ",
            "██╔══██╗",
            "██████╔╝",
            "██╔══██╗",
            "██████╔╝",
            "╚═════╝ ",
        ],
        'C' => [
            " ██████╗",
            "██╔════╝",
            "██║     ",
            "██║     ",
            "╚██████╗",
            " ╚═════╝",
        ],
        'D' => [
            "██████╗ ",
            "██╔══██╗",
            "██║  ██║",
            "██║  ██║",
            "██████╔╝",
            "╚═════╝ ",
        ],
        'E' => [
            "███████╗",
            "██╔════╝",
            "█████╗  ",
            "██╔══╝  ",
            "███████╗",
            "╚══════╝",
        ],
        'F' => [
            "███████╗",
            "██╔════╝",
            "█████╗  ",
            "██╔══╝  ",
            "██║     ",
            "╚═╝     ",
        ],
        '0' => [
            " ██████╗ ",
            "██╔═████╗",
            "██║██╔██║",
            "████╔╝██║",
            "╚██████╔╝",
            " ╚═════╝ ",
        ],
        '1' => [
            " ██╗",
            "███║",
            "╚██║",
            " ██║",
            " ██║",
            " ╚═╝",
        ],
        '2' => [
            "██████╗ ",
            "╚════██╗",
            " █████╔╝",
            "██╔═══╝ ",
            "███████╗",
            "╚══════╝",
        ],
        '3' => [
            "██████╗ ",
            "╚════██╗",
            " █████╔╝",
            " ╚═══██╗",
            "██████╔╝",
            "╚═════╝ ",
        ],
        '4' => [
            "██╗  ██╗",
            "██║  ██║",
            "███████║",
            "╚════██║",
            "     ██║",
            "     ╚═╝",
        ],
        '5' => [
            "███████╗",
            "██╔════╝",
            "███████╗",
            "╚════██║",
            "███████║",
            "╚══════╝",
        ],
        '6' => [
            " ██████╗ ",
            "██╔════╝ ",
            "███████╗ ",
            "██╔═══██╗",
            "╚██████╔╝",
            " ╚═════╝ ",
        ],
        '7' => [
            "███████╗",
            "╚════██║",
            "    ██╔╝",
            "   ██╔╝ ",
            "  ██╔╝  ",
            "  ╚═╝   ",
        ],
        '8' => [
            " █████╗ ",
            "██╔══██╗",
            "╚█████╔╝",
            "██╔══██╗",
            "╚█████╔╝",
            " ╚════╝ ",
        ],
        '9' => [
            " █████╗ ",
            "██╔══██╗",
            "╚██████║",
            " ╚═══██║",
            " █████╔╝",
            " ╚════╝ ",
        ],
        _ => return None,
    })
}

struct IntelSourceEval {
    reliability_letter: String,
    reliability_label: String,
    reliability_meter: f64,
    credibility_digit: String,
    credibility_label: String,
    credibility_meter: f64,
    footnote: String,
}

fn intel_source_evaluation(app: &App) -> IntelSourceEval {
    use argos_osint_core::osint::{
        best_credibility, wikipedia_rsp, InformationCredibility, SourceReliability,
    };
    let article = app.intel_articles.get(app.intel_sel);
    let domain = article
        .map(|row| row.source_domain.as_str())
        .unwrap_or("");
    let (reliability, entry) = match wikipedia_rsp::cached_index() {
        Some(index) => match index.lookup_domain(domain) {
            Some(entry) => (entry.status.reliability(), Some(entry.clone())),
            None => (SourceReliability::F, None),
        },
        None => {
            // Prefer a persisted letter from claims when the process cache is cold.
            let from_claim = app
                .intel_claims
                .iter()
                .find_map(|claim| SourceReliability::parse(&claim.reliability));
            (from_claim.unwrap_or(SourceReliability::F), None)
        }
    };
    let cred_values: Vec<InformationCredibility> = app
        .intel_claims
        .iter()
        .filter_map(|claim| InformationCredibility::from_u8(claim.info_credibility))
        .collect();
    let credibility = if cred_values.is_empty() {
        InformationCredibility::CannotBeJudged
    } else {
        best_credibility(&cred_values)
    };
    let code = format!("{}{}", reliability.as_str(), credibility.as_u8());
    let rsp = entry
        .as_ref()
        .map(|item| item.status.label())
        .or_else(|| {
            app.intel_claims
                .iter()
                .find(|claim| !claim.rsp_status.is_empty())
                .map(|claim| match claim.rsp_status.as_str() {
                    "gr" => "generally reliable",
                    "nc" | "m" => "no consensus",
                    "gu" => "generally unreliable",
                    "d" => "deprecated",
                    "b" => "blacklisted",
                    _ => "not in RSP",
                })
        })
        .unwrap_or("not in RSP");
    let year = entry
        .as_ref()
        .map(|item| item.last_year.as_str())
        .filter(|year| !year.is_empty())
        .unwrap_or("");
    let footnote = if year.is_empty() {
        format!("{code} · {domain} · {rsp}")
    } else {
        format!("{code} · {domain} · {rsp} · {year}")
    };
    IntelSourceEval {
        reliability_letter: reliability.as_str().into(),
        reliability_label: reliability.label().into(),
        reliability_meter: reliability.meter(),
        credibility_digit: credibility.as_u8().to_string(),
        credibility_label: credibility.label().into(),
        credibility_meter: credibility.meter(),
        footnote,
    }
}

fn draw_intel_tags(frame: &mut Frame, article: &argos_osint_core::store::AtlasArticleRow, area: Rect) {
    let block = pane(" tags ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // News classification categories only — never OSINT provider API names.
    let lines = vec![Line::from(Span::styled(
        format!(" {} ", atlas::category_name(&article.category)),
        theme::accent(),
    ))];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn intel_key_actors(app: &App) -> Vec<String> {
    let mut out = Vec::new();
    for claim in &app.intel_claims {
        let entity = claim.entity.trim();
        if entity.is_empty() || out.iter().any(|item| item == entity) {
            continue;
        }
        out.push(entity.to_string());
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn intel_locations(
    app: &App,
    article: &argos_osint_core::store::AtlasArticleRow,
) -> Vec<String> {
    // Event locations from claims only. Publisher country is shown in the preview,
    // not inferred as event geography.
    let mut out = Vec::new();
    let _ = article;
    for claim in &app.intel_claims {
        for value in [&claim.object, &claim.entity] {
            let value = value.trim();
            if value.len() == 2 && value.chars().all(|c| c.is_ascii_alphabetic()) {
                let label = atlas::country_label(value);
                if !out.iter().any(|item| item == &label) {
                    out.push(label);
                }
            }
        }
        if out.len() >= 8 {
            break;
        }
    }
    out
}

fn intel_timelines(
    app: &App,
    _article: &argos_osint_core::store::AtlasArticleRow,
) -> Vec<String> {
    // Event dates from claims. Publication date stays in the original preview.
    let mut out = Vec::new();
    for claim in &app.intel_claims {
        let stamp = claim.published_at.trim();
        if stamp.is_empty() {
            continue;
        }
        let label = atlas::friendly_date(stamp);
        if !out.iter().any(|item| item == &label) {
            out.push(label);
        }
        if out.len() >= 6 {
            break;
        }
    }
    out
}

fn intel_confidence_scores(app: &App) -> (f64, f64) {
    if app.intel_claims.is_empty() {
        return (0.0, 0.0);
    }
    let high = app
        .intel_claims
        .iter()
        .map(|claim| claim.confidence)
        .fold(0.0_f64, f64::max);
    let mean = app
        .intel_claims
        .iter()
        .map(|claim| claim.confidence)
        .sum::<f64>()
        / app.intel_claims.len() as f64;
    (high, mean)
}

fn intel_brief_preview_lines(
    article: &argos_osint_core::store::AtlasArticleRow,
    width: usize,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // Original article preview — never rewritten by enrichment.
    lines.push(Line::from(Span::styled(
        article.title.clone(),
        theme::text().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        format!(
            "Publisher {} · origin {} · published {}",
            article.source_name,
            article.country.to_ascii_uppercase(),
            atlas::friendly_date(&article.published_at)
        ),
        theme::dim(),
    )));
    if !article.url.trim().is_empty() {
        lines.push(Line::from(Span::styled(article.url.clone(), theme::dim())));
    }
    lines.push(Line::from(""));
    let summary = if article.description.trim().is_empty() {
        "No article summary stored.".to_string()
    } else {
        article.description.clone()
    };
    for chunk in wrap_text(&summary, width.max(1)) {
        lines.push(Line::from(Span::styled(chunk, theme::text())));
    }
    lines
}

fn intel_brief_full_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    // Loading is drawn as a centered Atlas-style spinner overlay, not as text lines.
    if intel_body_loading(app) {
        return lines;
    }
    match &app.intel_body {
        Some(body)
            if matches!(body.quality.as_str(), "complete" | "partial" | "uncertain")
                && !body.body_markdown.trim().is_empty() =>
        {
            if body.quality != "complete" {
                lines.push(Line::from(Span::styled(
                    format!("[{}] {}", body.quality, body.quality_rationale),
                    theme::dim(),
                )));
            }
            if app.intel_full_collapsed {
                lines.push(Line::from(Span::styled(
                    "[collapsed — press Reload to retrieve again]",
                    theme::dim(),
                )));
            } else {
                let md = super::markdown::markdown_lines(&body.body_markdown, width.max(1));
                for line in md {
                    lines.push(md_line_to_line(line));
                }
            }
        }
        Some(body) if !app.intel_body_message.is_empty() => {
            lines.push(Line::from(Span::styled(
                app.intel_body_message.clone(),
                theme::dim(),
            )));
        }
        Some(body) => {
            lines.push(Line::from(Span::styled(
                format!(
                    "Unavailable. {}",
                    if body.quality_rationale.is_empty() {
                        app.intel_body_message.as_str()
                    } else {
                        body.quality_rationale.as_str()
                    }
                ),
                theme::dim(),
            )));
            lines.push(Line::from(Span::styled(
                "[Press Reload to retry full article retrieval]",
                theme::dim(),
            )));
        }
        None => {}
    }
    lines
}

/// True while the focused brief is retrieving or refining the full article body.
pub(crate) fn intel_body_loading(app: &App) -> bool {
    let Some(article) = app.intel_articles.get(app.intel_sel) else {
        return false;
    };
    app.intel_body_running.contains(&article.id)
        || app
            .intel_body
            .as_ref()
            .is_some_and(|body| body.state == "running")
        || (app.intel_body.is_none() && !app.intel_body_message.is_empty())
}

/// Same centered spinner treatment as Atlas insights extraction.
fn draw_intel_body_loading(frame: &mut Frame, viewport: Rect, area: AbsRect, app: &App) {
    let Some((vis, _)) = intersect_abs(viewport, area) else {
        return;
    };
    frame.render_widget(Block::default().style(theme::text()), vis);
    stroke_clipped_box(frame, viewport, area, " full article ");

    let inner = AbsRect {
        x: area.x.saturating_add(1),
        y: area.y + 1,
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    };
    let Some((inner_vis, _)) = intersect_abs(viewport, inner) else {
        return;
    };
    let (label, detail) = intel_body_progress_lines(app);
    let mut lines = vec![Line::from(Span::styled(label, theme::dim()))];
    if let Some(detail) = detail {
        lines.push(Line::from(Span::styled(detail, theme::dim())));
    }
    let height = lines.len() as u16;
    let y = inner_vis.y + inner_vis.height.saturating_sub(height) / 2;
    let centered = Rect {
        x: inner_vis.x,
        y,
        width: inner_vis.width,
        height: height.min(inner_vis.height),
    };
    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center),
        centered,
    );
}

/// Mirror `insights_progress_lines`: braille spinner + phase label, optional detail.
fn intel_body_progress_lines(app: &App) -> (String, Option<String>) {
    let message = app.intel_body_message.trim();
    let lower = message.to_ascii_lowercase();
    let phase = if lower.contains("classif") || lower.contains("sponsored") {
        "Classifying article sections"
    } else if lower.contains("extract")
        || lower.contains("synthesis")
        || lower.contains("brief-relevant")
    {
        "Extracting article body"
    } else if lower.contains("saving") {
        "Saving article body"
    } else {
        "Retrieving full article"
    };
    let label = format!("{} {phase}", loading_spinner_frame());
    let detail = if message.is_empty()
        || message.eq_ignore_ascii_case(phase)
        || message.eq_ignore_ascii_case(&format!("{phase}…"))
    {
        None
    } else {
        Some(message.to_string())
    };
    (label, detail)
}

fn loading_spinner_frame() -> &'static str {
    const FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let frame = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| (elapsed.as_millis() / 80) as usize % FRAMES.len())
        .unwrap_or(0);
    FRAMES[frame]
}

fn intel_brief_reports_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if app.intel_jobs.is_empty() {
        lines.push(Line::from(Span::styled(
            "No Recon reports yet",
            theme::dim(),
        )));
        return lines;
    }
    for job in &app.intel_jobs {
        let mode = argos_osint_core::intel_recon::ReportMode::parse(&job.mode)
            .map(|m| m.title())
            .unwrap_or(job.mode.as_str());
        lines.push(Line::from(Span::styled(
            format!(
                "{mode} · r{} · {} · {}",
                job.revision, job.state, job.updated_at
            ),
            theme::text().add_modifier(Modifier::BOLD),
        )));
        let sections = if app
            .intel_sections
            .first()
            .is_some_and(|s| s.job_id == job.id)
        {
            app.intel_sections.clone()
        } else {
            Vec::new()
        };
        if sections.is_empty() {
            lines.push(Line::from(Span::styled(
                format!(
                    "  {}/{} sections · stage {}",
                    job.sections_done, job.sections_total, job.stage
                ),
                theme::dim(),
            )));
            continue;
        }
        for section in &sections {
            let marker = match section.status.as_str() {
                "complete" => "✓",
                "running" => "…",
                "waiting" | "stale" => "○",
                "failed" => "!",
                _ => "·",
            };
            lines.push(Line::from(Span::styled(
                format!("{marker} {}", section.title),
                theme::accent(),
            )));
            if section.status == "waiting" && !section.waiting_on.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  waiting for {}", section.waiting_on),
                    theme::dim(),
                )));
            }
            let collapsed_key = format!("{}:{}", job.id, section.section_key);
            if app.intel_collapsed_sections.contains(&collapsed_key) {
                lines.push(Line::from(Span::styled("  [collapsed]", theme::dim())));
                continue;
            }
            if !section.markdown.trim().is_empty() {
                let md = super::markdown::markdown_lines(&section.markdown, width.max(1));
                for line in md {
                    lines.push(md_line_to_line(line));
                }
            } else if section.status == "running" {
                lines.push(Line::from(Span::styled(
                    "  synthesizing…",
                    theme::dim(),
                )));
            }
        }
        lines.push(Line::from(""));
    }
    lines
}

fn md_line_to_line(line: super::markdown::MdLine) -> Line<'static> {
    if line.pieces.is_empty() {
        return Line::from("");
    }
    Line::from(
        line.pieces
            .into_iter()
            .map(|piece| Span::styled(piece.text, super::markdown::style(piece.tone)))
            .collect::<Vec<_>>(),
    )
}

fn intel_hit(app: &App, body: Rect, x: u16, y: u16) -> Option<Target> {
    if app.intel_page == IntelPage::Briefing {
        let (_left, center, right) = intel_briefing_areas(body);
        if let Some(layout) = intel_center_layout(app, center) {
            if abs_contains(layout.reload, x, y) && contains(center, x, y) {
                return Some(Target::Button(ButtonId::IntelBodyRefresh));
            }
        }
        let rows = intel_briefing_right_rows(app, right);
        if contains(rows[3], x, y) {
            return Some(Target::Button(ButtonId::IntelRecon));
        }
        if contains(rows[4], x, y) {
            // Clicking the jobs pane focuses Recon for job controls.
            return Some(Target::Button(ButtonId::IntelJobOpen));
        }
        return None;
    }
    let (tabs, title_row, _hero, search, list) = intel_bulletin_areas(body);
    let tab_slots = button_areas(tabs, INTEL_CATEGORIES.len());
    for (index, rect) in tab_slots.into_iter().enumerate() {
        if contains(rect, x, y) {
            return Some(Target::IntelTab(index));
        }
    }
    if contains(title_row, x, y) {
        return Some(Target::Button(ButtonId::IntelDay));
    }
    if contains(search, x, y) {
        return Some(Target::Field(FieldId::IntelSearch));
    }
    if in_pane(list, x, y) {
        let index = app.scrolls.intel_list as usize + (y - list.y - 1) as usize;
        if index < app.intel_articles.len() {
            return Some(Target::IntelArticle(index));
        }
    }
    None
}

fn draw_atlas(frame: &mut Frame, app: &App, area: Rect) {
    if app.atlas_page == AtlasPage::Runs {
        draw_atlas_runs(frame, app, area);
    } else {
        draw_atlas_live(frame, app, area);
    }
}

fn draw_atlas_insights(frame: &mut Frame, app: &App, area: Rect) {
    if atlas_extracting(app) {
        let block = pane(" insights ");
        let inner = inset(area);
        frame.render_widget(block, area);
        let (label, metric) = insights_progress_lines(app);
        let mut lines = vec![Line::from(Span::styled(label, theme::dim()))];
        if let Some(metric) = metric {
            lines.push(Line::from(Span::styled(metric, theme::dim())));
        }
        let height = lines.len() as u16;
        let y = inner.y + inner.height.saturating_sub(height) / 2;
        let centered = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: height.min(inner.height),
        };
        frame.render_widget(
            Paragraph::new(lines).alignment(Alignment::Center),
            centered,
        );
        return;
    }
    let width = inset(area).width as usize;
    let room = inset(area).height.saturating_sub(2) as usize;
    let table = atlas_insights::insight_table_lines(&app.atlas_stats.insights);
    let (head, rows) = if table.len() <= 2 {
        (table, Vec::new())
    } else {
        let mut table = table;
        let rows = table.split_off(2);
        (table, rows)
    };
    let start = (app.scrolls.insights as usize).min(rows.len().saturating_sub(room.max(1)));
    let mut lines: Vec<Line> = head
        .into_iter()
        .map(|line| Line::from(Span::styled(center_text(&line, width), theme::dim())))
        .collect();
    lines.extend(rows.into_iter().skip(start).take(room.max(1)).map(|line| {
        Line::from(Span::styled(center_text(&line, width), theme::text()))
    }));
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane(" insights "))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// True while the live pipeline is in the insight-extraction step.
pub(crate) fn atlas_extracting(app: &App) -> bool {
    app.atlas_pause.is_some()
        && (app.atlas_insight_progress.is_some() || app.atlas_status == "Extracting insights")
}

fn insights_progress_lines(app: &App) -> (String, Option<String>) {
    let label = format!("{} Extracting insights", loading_spinner_frame());
    let metric = app
        .atlas_insight_progress
        .filter(|(_, total)| *total > 0)
        .map(|(done, total)| format!("{done} / {total}"));
    (label, metric)
}

fn draw_atlas_live(frame: &mut Frame, app: &App, area: Rect) {
    let (actions, table, insights, feed) = atlas_live_areas(area);
    let buttons = button_areas(actions, 3);
    let auto = atlas_auto_label(app);
    draw_button(frame, app, ButtonId::AtlasRuns, "History", buttons[0]);
    draw_button(frame, app, ButtonId::AtlasAuto, &auto, buttons[1]);
    draw_button(
        frame,
        app,
        ButtonId::AtlasRun,
        atlas_run_label(app),
        buttons[2],
    );
    let width = inset(table).width as usize;
    let room = inset(table).height.saturating_sub(1) as usize;
    let header = "Country                         Tier   Temp   Volume  Articles  Share";
    let mut lines = vec![Line::from(Span::styled(
        center_text(header, width),
        theme::dim(),
    ))];
    let data: Vec<String> = if app.atlas_stats.origins.is_empty() {
        vec!["No country statistics yet.".into()]
    } else {
        let rows: Vec<&atlas::OriginStat> = app
            .atlas_stats
            .origins
            .iter()
            .filter(|row| row.articles > 0 || !app.atlas_stats.scored)
            .collect();
        if rows.is_empty() {
            vec!["No country statistics yet.".into()]
        } else {
            rows.into_iter()
                .map(|row| {
                    let tier = if row.tier == 0 {
                        "-".into()
                    } else {
                        row.tier.to_string()
                    };
                    format!(
                        "{:<32} {:<6} {:<6.2} {:<7} {:<9} {:.0}%",
                        atlas::country_label(&row.country),
                        tier,
                        row.temperature,
                        row.volume,
                        row.articles,
                        app.atlas_stats.share(row.articles) * 100.0
                    )
                })
                .collect()
        }
    };
    let start = (app.scrolls.origins as usize).min(data.len().saturating_sub(room.max(1)));
    for line in data.into_iter().skip(start).take(room.max(1)) {
        let empty = line == "No country statistics yet.";
        lines.push(Line::from(Span::styled(
            center_text(&line, width),
            if empty {
                theme::dim()
            } else {
                theme::text()
            },
        )));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane(" origins "))
            .wrap(Wrap { trim: false }),
        table,
    );
    draw_atlas_insights(frame, app, insights);
    let feed_width = inset(feed).width as usize;
    let feed_lines = if app.atlas_feed.is_empty() {
        vec![Line::from(Span::styled(
            "Headlines from this session appear here. They are not saved.",
            theme::dim(),
        ))]
    } else {
        app.atlas_feed
            .iter()
            .enumerate()
            .skip(app.scrolls.atlas_feed as usize)
            .map(|(index, article)| {
                let style = if index == app.atlas_feed_sel {
                    theme::selected()
                } else {
                    theme::text()
                };
                Line::from(Span::styled(
                    fit(
                        &format!(
                            "{}  {}  {}",
                            atlas::country_label(&article.country),
                            atlas::publisher_and_author(
                                &article.source_name,
                                &article.source_domain,
                                &article.author,
                            )
                            .0,
                            article.title
                        ),
                        feed_width,
                    ),
                    style,
                ))
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(feed_lines).block(pane(" headlines ")), feed);
}

fn draw_atlas_runs(frame: &mut Frame, app: &App, area: Rect) {
    if app.atlas_news {
        draw_atlas_news(frame, app, area);
        return;
    }
    let (actions, map, stats, cycles) = atlas_runs_areas(area);
    draw_map_or_hold(frame, app, map);
    draw_atlas_cycle_stats(frame, app, stats);
    let width = inset(cycles).width as usize;
    let lines = if app.atlas_runs.is_empty() {
        vec![Line::from(Span::styled(
            "No news cycles yet.",
            theme::dim(),
        ))]
    } else {
        app.atlas_runs
            .iter()
            .enumerate()
            .skip(app.scrolls.atlas_runs as usize)
            .map(|(index, run)| {
                news_cycle_line(run, width, index == app.atlas_run_sel)
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines).block(pane(" news cycle ")), cycles);
    let buttons = button_areas(actions, 2);
    let live = atlas_history_live_label(app);
    draw_button(frame, app, ButtonId::AtlasLive, &live, buttons[0]);
    draw_button(frame, app, ButtonId::AtlasDelete, "Delete", buttons[1]);
}

/// Date on the left, pipeline state on the right.
fn news_cycle_line(run: &argos_osint_core::store::AtlasRunRow, width: usize, selected: bool) -> Line<'static> {
    let style = if selected {
        theme::selected()
    } else {
        theme::text()
    };
    let when = atlas::friendly_date(&run.started_at);
    let status = run.state.trim();
    let status = if status.is_empty() { "unknown" } else { status };
    let gap = width
        .saturating_sub(when.chars().count())
        .saturating_sub(status.chars().count());
    if gap == 0 {
        return Line::from(Span::styled(
            fit(&format!("{when} {status}"), width),
            style,
        ));
    }
    Line::from(vec![
        Span::styled(when, style),
        Span::styled(" ".repeat(gap), style),
        Span::styled(status.to_string(), style),
    ])
}

fn draw_atlas_cycle_stats(frame: &mut Frame, app: &App, area: Rect) {
    let width = inset(area).width as usize;
    let room = inset(area).height.saturating_sub(3) as usize;
    let stats = selected_cycle_stats(app);
    let origins: Vec<&atlas::OriginStat> = stats
        .origins
        .iter()
        .filter(|row| row.articles > 0)
        .collect();
    let summary = format!(
        "Articles {}  {}",
        stats.article_total(),
        atlas_insights::insight_stats_line(&stats.insights)
    );
    let header = "Country                         Tier   Temp   Volume  Articles  Share";
    let mut lines = vec![
        Line::from(Span::styled(fit(&summary, width), theme::dim())),
        Line::from(""),
        Line::from(Span::styled(header.to_string(), theme::dim())),
    ];
    let data: Vec<String> = if origins.is_empty() {
        vec!["No country statistics for this cycle.".into()]
    } else {
        origins
            .iter()
            .map(|row| {
                let tier = if row.tier == 0 {
                    "-".into()
                } else {
                    row.tier.to_string()
                };
                format!(
                    "{:<32} {:<6} {:<6.2} {:<7} {:<9} {:.0}%",
                    atlas::country_label(&row.country),
                    tier,
                    row.temperature,
                    row.volume,
                    row.articles,
                    stats.share(row.articles) * 100.0
                )
            })
            .collect()
    };
    let start = (app.scrolls.origins as usize).min(data.len().saturating_sub(room.max(1)));
    for line in data.into_iter().skip(start).take(room.max(1)) {
        lines.push(Line::from(Span::styled(
            fit(&line, width),
            if origins.is_empty() {
                theme::dim()
            } else {
                theme::text()
            },
        )));
    }
    frame.render_widget(
        Paragraph::new(lines).block(pane(" stats ")),
        area,
    );
}

fn draw_map_or_hold(frame: &mut Frame, app: &App, area: Rect) {
    if app.atlas_map_hold {
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new("Loading country")
                .alignment(Alignment::Center)
                .style(theme::dim())
                .block(pane(" map ")),
            area,
        );
        return;
    }
    super::map::draw_world_map(frame, app, area);
}

fn news_tab_label(app: &App) -> String {
    let when = app
        .atlas_runs
        .iter()
        .find(|run| run.id == app.atlas_news_run)
        .map(|run| atlas::friendly_date(&run.started_at))
        .unwrap_or_else(|| "unknown".into());
    format!("news: {when} - {} articles", app.atlas_articles.len())
}

fn draw_atlas_news(frame: &mut Frame, app: &App, area: Rect) {
    let (map, tabs, list) = atlas_news_areas(area);
    draw_map_or_hold(frame, app, map);
    let tab_slots = button_areas(tabs, 2);
    if let (Some(world), Some(news)) = (tab_slots.first(), tab_slots.get(1)) {
        draw_button(frame, app, ButtonId::AtlasWorld, "world map", *world);
        draw_button_state(
            frame,
            app,
            ButtonId::AtlasNews,
            &news_tab_label(app),
            *news,
            true,
        );
    }
    let width = inset(list).width as usize;
    let lines = if app.atlas_articles.is_empty() {
        vec![Line::from(Span::styled(
            "No saved articles for this run.",
            theme::dim(),
        ))]
    } else {
        app.atlas_articles
            .iter()
            .enumerate()
            .skip(app.scrolls.atlas_news as usize)
            .map(|(index, article)| {
                news_feed_line(
                    article,
                    width,
                    index == app.atlas_article_sel,
                    app.claim_mark.as_deref() == Some(article.id.as_str()),
                )
            })
            .collect()
    };
    frame.render_widget(Paragraph::new(lines).block(pane(" news ")), list);
}

fn atlas_row_at(area: Rect, scroll: u16, y: u16) -> Option<usize> {
    let inner = inset(area);
    if !contains(inner, inner.x, y) && y < inner.y {
        return None;
    }
    if y < inner.y || y >= inner.y + inner.height {
        return None;
    }
    Some(scroll as usize + (y - inner.y) as usize)
}

fn atlas_hit(app: &App, body: Rect, x: u16, y: u16) -> Option<Target> {
    if app.atlas_page == AtlasPage::Runs {
        if app.atlas_news {
            let (_map, tabs, list) = atlas_news_areas(body);
            if contains(tabs, x, y) {
                let slots = button_areas(tabs, 2);
                return Some(Target::Button(if contains(slots[0], x, y) {
                    ButtonId::AtlasWorld
                } else {
                    ButtonId::AtlasNews
                }));
            }
            if contains(list, x, y) {
                let index = atlas_row_at(list, app.scrolls.atlas_news, y)?;
                return (index < app.atlas_articles.len()).then_some(Target::AtlasArticle(index));
            }
            return None;
        }
        let (actions, _map, stats, cycles) = atlas_runs_areas(body);
        if contains(actions, x, y) {
            let slots = button_areas(actions, 2);
            return Some(Target::Button(if contains(slots[0], x, y) {
                ButtonId::AtlasLive
            } else {
                ButtonId::AtlasDelete
            }));
        }
        if contains(stats, x, y) {
            return Some(Target::AtlasCycleStats);
        }
        if contains(cycles, x, y) {
            let index = atlas_row_at(cycles, app.scrolls.atlas_runs, y)?;
            return (index < app.atlas_runs.len()).then_some(Target::AtlasHistory(index));
        }
        return None;
    }
    let (actions, _table, _insights, feed) = atlas_live_areas(body);
    if contains(actions, x, y) {
        let buttons = button_areas(actions, 3);
        return Some(Target::Button(if contains(buttons[0], x, y) {
            ButtonId::AtlasRuns
        } else if contains(buttons[1], x, y) {
            ButtonId::AtlasAuto
        } else {
            ButtonId::AtlasRun
        }));
    }
    if contains(feed, x, y) {
        let index = atlas_row_at(feed, app.scrolls.atlas_feed, y)?;
        return (index < app.atlas_feed.len()).then_some(Target::AtlasFeed(index));
    }
    None
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
            .block(pane(" host "))
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
        log_rows(app, inset(log).width as usize)
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane(" event log "))
            .scroll((app.scrolls.log, 0)),
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
        Overlay::IntelRecon => {
            let mode = app.intel_recon_mode();
            let mut lines = vec![
                mode.title().to_string(),
                mode.description().to_string(),
                String::new(),
                "Sections".into(),
            ];
            for section in intel_recon::section_plan(mode) {
                let mark = if app.intel_recon_section_enabled(mode, section.key) {
                    "[x]"
                } else {
                    "[ ]"
                };
                lines.push(format!("{mark} {}", section.title));
            }
            lines.join("\n")
        }
        Overlay::Palette => app
            .palette_items()
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>()
            .join("\n"),
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
                lines.push(format!(
                    "Recorded: {}",
                    atlas::friendly_date(&memory.created_at)
                ));
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
        None => "Home\n\n↑↓ or j/k select an application\nEnter opens it\n1 Intel · 2 Atlas · 3 Brain · 4 Recon · 5 OSINT · 6 Providers · 7 System\nCtrl+Tab cycles apps · Ctrl+Shift+Tab goes back\nCtrl+K command palette · ? help · Esc closes this card\nCtrl+C quits when nothing is running · Ctrl+Q quits from anywhere",
        Some(ModuleId::Intel) => "Intel\n\nBulletin board browses Atlas-stored headlines by classification\nSix tabs: Geopolitical Economic Military Information Stability Tech\nThe day button filters by Atlas news-cycle run day\nSearch filters title, description, source, and URL\n↑↓ select a story · the hero updates with the selection\nEnter opens Briefing Focus for that article\nBriefing shows preview, full article, REPORTS, extracted actors, and confidence\nRecon opens Verify / Explain / Assess Outlook / Full Assessment\nJobs pane tracks report progress under the Recon button\nEsc returns from briefing to bulletin, or from bulletin to home",
        Some(ModuleId::Atlas) => "Atlas\n\nNews cycle is the view that opens. Go Live shows the pipeline\nRun starts the pipeline. Pause parks it after the current request\nResume continues that run. Ctrl+C pauses\nAuto Run starts the pipeline now and again every 90 minutes until it is turned off\nThe button shows when the next run starts. A manual run moves that time out by 90 minutes\nThe table shows country heat. The feed lists headlines from this session\n↑↓ move through headlines · the wheel and Ctrl+U/D scroll that list\nEnter or click opens the selected headline\nFailed requests, including rate limits, are written to the System event log\nEnter on a ▸ error there opens the full API response\nNews cycle lists saved cycles by date and status. Enter or click opens that cycle's news feed\nStats for the selected cycle sit under the map, left of the list\nClick the stats pane, then ↑↓ or the wheel scrolls the country table\nThe world map sits above those panes and takes most of the view\nGo Live and Delete sit between the map and those panes. When auto run is on, Go Live counts down\nThe map follows the selected news cycle. It does not take keys or clicks\nTier 1 and 2 countries are named in full. Tier 3 shows the country code\nAnother news cycle row recolours the map and replaces the stats\nThe news list shows the title, then publisher, country code, and category\nEnter or click opens the article and zooms the map to its country\nWorld map restores the news cycle list and zooms back out\nDelete removes the selected cycle. Backspace does the same when a cycle is focused\nEsc on the news feed or on Live returns to news cycle\nEsc on news cycle returns home",
        Some(ModuleId::Recon) if !app.recon_chat => "Recon investigations\n\nThe list is the most recent investigations\n↑↓ move · Enter opens the transcript\nNew starts an investigation · Delete removes the selected one\nType to search titles\nEsc returns home · Ctrl+N new investigation",
        Some(ModuleId::Recon) => "Recon chat\n\nEnter sends · Shift+Enter inserts a line · / opens commands\nTab moves between the transcript and the prompt\n↑↓ select a message, recon log, or tool\n←→ or h/l fold the selected recon log or tool\nEnter toggles that fold · f opens the full text\n◉ brain opens the memories Synthesis used\nrecall: off skips insight extraction. recall: on writes claims for later answers\nCtrl+K command palette · Ctrl+U/Ctrl+D scroll\nEsc returns to investigations · Ctrl+C cancels a running turn\nCtrl+N new thread · Alt+←/→ recent threads",
        Some(ModuleId::System) => "System\n\nRefresh hardware re-reads the host profile\nThe event log keeps errors, run stages, and tool results for 24 hours\n↑↓ select a line · Enter or click the arrow folds a tool result\nCtrl+U/Ctrl+D and the wheel scroll the log\nEsc returns home",
        Some(ModuleId::Brain) => "Brain\n\nMemories lists saved insights. Find filters that list\nEnter opens a recon path, or a claim path for a news insight\nThe path sits above a summary of the graph\nThe first visit asks Synthesis to write the summary and saves it\nThe summary says why the concluding insight is a fact or an inference\nClick a recon path to open its source thread\nClick an article on a claim path to open that news cycle\nEsc returns to memories\nCreate replaces the list with the form. Save stores the memory\nEsc returns home from the list · ? opens this card",
        Some(ModuleId::Providers) => "Providers\n\nEach account tab stores that provider only\nDefaults sets Recon and Synthesis separately\nProvider and Model open the accounts and models that connection can use\n↑↓ choose · Enter selects · Esc closes the list\nEsc returns home · ? opens this card",
        _ => "Controls\n\nTab moves between fields and buttons\nCtrl+Tab cycles app tabs · Ctrl+Shift+Tab goes back\nEnter activates the focused control\n↑↓ move through lists\nCtrl+U/Ctrl+D and the wheel scroll the pane under the pointer\nTyping works only in a focused field\nEsc returns home · ? opens this card",
    }
}

fn cover(frame: &mut Frame, area: Rect) {
    frame.render_widget(Clear, area);
    if area.width == 0 || area.height == 0 {
        return;
    }
    let fill = " ".repeat(area.width as usize);
    let lines = vec![Line::from(Span::styled(fill, theme::card_text())); area.height as usize];
    frame.render_widget(Paragraph::new(lines), area);
}

fn popup_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let center_table =
        matches!(&app.overlay, Overlay::Block { title, .. } if title.starts_with("Run "));
    let mut in_table = false;
    popup_text(app)
        .lines()
        .map(|line| {
            if center_table && (in_table || line.starts_with("Country")) {
                in_table = !line.is_empty();
                Line::from(center_text(line, width))
            } else {
                Line::from(line.to_string())
            }
        })
        .collect()
}

fn draw_overlay(frame: &mut Frame, app: &App) {
    if matches!(app.overlay, Overlay::Palette) {
        draw_palette(frame, app);
        return;
    }
    if let Overlay::Choice(kind) = app.overlay {
        draw_choice(frame, app, kind);
        return;
    }
    if app.overlay == Overlay::IntelRecon {
        draw_intel_recon_popup(frame, app);
        return;
    }
    let area = popup_area(frame.area());
    cover(frame, area);
    let title = match &app.overlay {
        Overlay::Help => " Shortcuts ",
        Overlay::Memories { .. } => " Memory ",
        Overlay::Block { .. } => " Detail ",
        Overlay::Choice(_) | Overlay::IntelRecon | Overlay::Palette | Overlay::None => " ",
    };
    let run_card = atlas_run_card(app);
    let body = if run_card {
        let button = run_news_rect(area);
        Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: button.y.saturating_sub(area.y).max(3),
        }
    } else {
        area
    };
    let inner_width = inset(body).width as usize;
    frame.render_widget(
        Paragraph::new(popup_lines(app, inner_width))
            .style(theme::card_text())
            .block(theme::card(title))
            .scroll((app.scrolls.popup, 0))
            .wrap(Wrap { trim: false }),
        body,
    );
    let close = Rect {
        x: area.x + area.width.saturating_sub(8),
        y: area.y,
        width: 8.min(area.width),
        height: 1,
    };
    if run_card {
        let button = run_news_rect(area).intersection(frame.area());
        draw_button(frame, app, ButtonId::AtlasNewsFeed, "News feed", button);
        frame.render_widget(
            Paragraph::new(" delete ").style(theme::card_accent()),
            run_delete_rect(area),
        );
    }
    frame.render_widget(Paragraph::new(" close ").style(theme::card_accent()), close);
}

fn draw_palette(frame: &mut Frame, app: &App) {
    let area = popup_area(frame.area());
    cover(frame, area);
    frame.render_widget(Paragraph::new("").block(theme::card(" commands ")), area);
    let inner = inset(area);
    if inner.height == 0 {
        return;
    }
    frame.render_widget(
        Paragraph::new(format!("▎{}", app.palette_query)).style(theme::user_message()),
        Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        },
    );
    let start = app.scrolls.popup as usize;
    let room = inner.height.saturating_sub(1) as usize;
    for (index, item) in app
        .palette_items()
        .into_iter()
        .enumerate()
        .skip(start)
        .take(room)
    {
        let selected = index == app.palette_sel;
        let mark = if selected { "▸ " } else { "  " };
        frame.render_widget(
            Paragraph::new(fit(&format!("{mark}{}", item.label), inner.width as usize)).style(
                if selected {
                    theme::selected()
                } else {
                    theme::card_text()
                },
            ),
            Rect {
                x: inner.x,
                y: inner.y + 1 + (index - start) as u16,
                width: inner.width,
                height: 1,
            },
        );
    }
}

struct IntelReconPopupLayout {
    tabs: Vec<Rect>,
    sections: Vec<Rect>,
    start: Rect,
    section_room: usize,
}

fn intel_recon_popup_layout(area: Rect, app: &App) -> IntelReconPopupLayout {
    let inner = inset(area);
    let rows = split_vertical(
        inner,
        [
            Constraint::Length(ACTION_H), // mode tabs
            Constraint::Length(3),       // description
            Constraint::Min(4),          // section toggles
            Constraint::Length(ACTION_H), // start
        ],
    );
    let tabs = button_areas(rows[0], ReportMode::all().len());
    let section_area = rows[2];
    let room = section_area.height.max(1) as usize;
    let plan = intel_recon::section_plan(app.intel_recon_mode());
    let start_scroll = app.scrolls.popup as usize;
    let mut sections = Vec::new();
    for visible in 0..room.min(plan.len().saturating_sub(start_scroll)) {
        sections.push(Rect {
            x: section_area.x,
            y: section_area.y + visible as u16,
            width: section_area.width,
            height: 1,
        });
    }
    // Map visible rows back to absolute section indices for hit testing.
    let section_rects: Vec<Rect> = (0..plan.len())
        .map(|index| {
            if index < start_scroll || index >= start_scroll + room {
                Rect::default()
            } else {
                Rect {
                    x: section_area.x,
                    y: section_area.y + (index - start_scroll) as u16,
                    width: section_area.width,
                    height: 1,
                }
            }
        })
        .collect();
    let _ = sections;
    let start_row = rows[3];
    let start_w = 14u16.min(start_row.width.saturating_sub(2)).max(8);
    let start = Rect {
        x: start_row.x + start_row.width.saturating_sub(start_w) / 2,
        y: start_row.y,
        width: start_w,
        height: start_row.height,
    };
    IntelReconPopupLayout {
        tabs,
        sections: section_rects,
        start,
        section_room: room,
    }
}

pub fn intel_recon_section_room(app: &App) -> usize {
    let area = intel_recon_popup_area(app.screen);
    intel_recon_popup_layout(area, app).section_room
}

fn draw_intel_recon_popup(frame: &mut Frame, app: &App) {
    let area = intel_recon_popup_area(frame.area());
    cover(frame, area);
    frame.render_widget(Paragraph::new("").block(theme::card(" Recon ")), area);
    let close = Rect {
        x: area.x + area.width.saturating_sub(8),
        y: area.y,
        width: 8.min(area.width),
        height: 1,
    };
    frame.render_widget(Paragraph::new(" close ").style(theme::card_accent()), close);

    let layout = intel_recon_popup_layout(area, app);
    let modes = ReportMode::all();
    let mode = app.intel_recon_mode();
    draw_tabs(
        frame,
        Rect {
            x: layout.tabs.first().map(|r| r.x).unwrap_or(area.x),
            y: layout.tabs.first().map(|r| r.y).unwrap_or(area.y),
            width: layout
                .tabs
                .last()
                .map(|r| r.x + r.width)
                .unwrap_or(area.x)
                .saturating_sub(layout.tabs.first().map(|r| r.x).unwrap_or(area.x)),
            height: ACTION_H,
        },
        modes.iter().enumerate().map(|(index, item)| {
            (
                index,
                item.title().to_string(),
                index == app.intel_recon_tab,
                app.focus == Target::IntelReconTab(index)
                    || matches!(app.intel_recon_focus, IntelReconFocus::Tab(i) if i == index),
            )
        }),
    );

    let inner = inset(area);
    let rows = split_vertical(
        inner,
        [
            Constraint::Length(ACTION_H),
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(ACTION_H),
        ],
    );
    let blurb = format!(
        "{}\nOutlook horizon default: 30 days · toggle sections to customize the report",
        mode.description()
    );
    frame.render_widget(
        Paragraph::new(blurb)
            .style(theme::card_dim())
            .wrap(Wrap { trim: false }),
        rows[1],
    );

    let plan = intel_recon::section_plan(mode);
    let start = app.scrolls.popup as usize;
    let room = layout.section_room.max(1);
    for (offset, section) in plan.iter().enumerate().skip(start).take(room) {
        let index = offset;
        let enabled = app.intel_recon_section_enabled(mode, section.key);
        let focused = app.focus == Target::IntelReconSection(index)
            || matches!(app.intel_recon_focus, IntelReconFocus::Section(i) if i == index);
        let mark = if enabled { "[x]" } else { "[ ]" };
        let text = fit(
            &format!("{mark} {}", section.title),
            rows[2].width.max(1) as usize,
        );
        frame.render_widget(
            Paragraph::new(text).style(if focused {
                theme::selected()
            } else if enabled {
                theme::card_text()
            } else {
                theme::card_dim()
            }),
            Rect {
                x: rows[2].x,
                y: rows[2].y + (index - start) as u16,
                width: rows[2].width,
                height: 1,
            },
        );
    }

    draw_button(
        frame,
        app,
        ButtonId::IntelReconStart,
        "Start",
        layout.start,
    );
}

fn draw_choice(frame: &mut Frame, app: &App, kind: ChoiceKind) {
    let area = popup_area(frame.area());
    cover(frame, area);
    let title = match kind {
        ChoiceKind::Provider => format!(" {} provider ", app.defaults_role.label()),
        ChoiceKind::Model => format!(" {} model ", app.defaults_role.label()),
        ChoiceKind::IntelDay => " news cycle day ".into(),
    };
    frame.render_widget(Paragraph::new("").block(theme::card(&title)), area);
    let inner = inset(area);
    let mut y = inner.y;
    let mut height = inner.height;
    if !app.choice_note.is_empty() && height > 0 {
        frame.render_widget(
            Paragraph::new(fit(
                &app.choice_note.replace('\n', " "),
                inner.width as usize,
            ))
            .style(theme::card_dim()),
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
        let empty = match kind {
            ChoiceKind::Model => "No models for this account yet.",
            ChoiceKind::IntelDay => "No Atlas news-cycle days yet.",
            ChoiceKind::Provider => "No connected account yet. Local is always listed.",
        };
        frame.render_widget(
            Paragraph::new(empty).style(theme::card_dim()),
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
                theme::card_text()
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
    frame.render_widget(Paragraph::new(" close ").style(theme::card_accent()), close);
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
                directive(
                    "d1",
                    "Establish the subject's identity and public roles",
                    &["person_name", "org_name", "url"],
                ),
                directive(
                    "d2",
                    "Find the subject's official online accounts and websites",
                    &["handle", "domain", "url"],
                ),
                directive(
                    "d3",
                    "Find organizations affiliated with the subject and their contact domains",
                    &["org_name", "domain", "email"],
                ),
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
            bindings: vec![
                Binding {
                    kind: "handle".into(),
                    value: "janeroe".into(),
                    evidence_id: "call-s1".into(),
                    step_id: "s1".into(),
                    qualifier: "github".into(),
                    inferred: false,
                    unverified: false,
                    source_tool: String::new(),
                },
                Binding {
                    kind: "handle".into(),
                    value: "janeroe".into(),
                    evidence_id: "call-s1".into(),
                    step_id: "s1".into(),
                    qualifier: "facebook".into(),
                    inferred: true,
                    unverified: false,
                    source_tool: String::new(),
                },
                Binding {
                    kind: "handle".into(),
                    value: "janeroe".into(),
                    evidence_id: "d2".into(),
                    step_id: String::new(),
                    qualifier: "twitter".into(),
                    inferred: false,
                    unverified: true,
                    source_tool: String::new(),
                },
            ],
            binding_notes: vec!["s1 firecrawl_search: rules found 1; Recon model added 0".into()],
            fallback_requests: vec![
                "hunter_email_finder failed. Recon chose firecrawl_scrape as s3.".into(),
            ],
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
        assert!(
            block
                .title
                .starts_with("Recon log · tool picker (decisions)"),
            "{}",
            block.title
        );
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
        assert!(
            !block.body.contains("0.87"),
            "probabilities stay out of the row"
        );
        assert!(
            !block.body.contains("Jane Roe role"),
            "raw observations stay out of the decision row"
        );
        let logged = tool_result_log(&calls[0]).unwrap();
        assert_eq!(
            logged.summary,
            "Firecrawl search completed · live · 2 results"
        );
        assert!(logged.detail.contains("Jane Roe role"));
        assert!(logged.detail.contains("https://example.test/jane"));
        let markdown = "Jane Example jane@acmerobotics.com ".repeat(80);
        let mut page = calls[0].clone();
        page.tool_id = "firecrawl_scrape".into();
        page.result.as_mut().unwrap().tool_id = "firecrawl_scrape".into();
        page.result.as_mut().unwrap().observations = serde_json::json!({
            "title": "Contact",
            "url": "https://acmerobotics.com/contact",
            "markdown": markdown,
            "evidence_form": "page",
        });
        let logged = tool_result_log(&page).unwrap();
        assert!(logged
            .detail
            .contains("Contact · https://acmerobotics.com/contact"));
        assert!(!logged.detail.contains("jane@acmerobotics.com"));
        let mut job = page.clone();
        job.tool_id = "firecrawl_crawl".into();
        job.result.as_mut().unwrap().observations = serde_json::json!({
            "pages": [
                {"title": "About", "url": "https://acmerobotics.com/about", "markdown": markdown},
                {"title": "Contact", "url": "https://acmerobotics.com/contact", "markdown": markdown}
            ],
            "evidence_form": "page",
        });
        let logged = tool_result_log(&job).unwrap();
        assert!(logged.detail.contains("2 pages"));
        assert!(logged
            .detail
            .contains("About · https://acmerobotics.com/about"));
        assert!(!logged.detail.contains("jane@acmerobotics.com"));
        let mut extract = page.clone();
        extract.tool_id = "firecrawl_extract".into();
        extract.result.as_mut().unwrap().observations = serde_json::json!({
            "org_name": "Acme Robotics",
            "domain": "acmerobotics.com",
            "emails": ["jane@acmerobotics.com"],
            "people": [{"name": "Jane Example", "title": "CEO"}],
            "address": "100 Market Street\nSan Francisco",
            "url": "https://acmerobotics.com/about",
            "evidence_form": "extract",
        });
        let logged = tool_result_log(&extract).unwrap();
        assert!(logged
            .detail
            .contains("Acme Robotics · acmerobotics.com · 1 email · 1 person · address"));
        assert!(!logged.detail.contains("100 Market Street"));
        assert!(!logged.detail.contains("Jane Example"));
    }

    #[test]
    fn home_pads_titles_and_application_order() {
        let area = Rect::new(0, 0, 120, 40);
        let rows = home_rows(area, 0);
        let logo_at = rows
            .iter()
            .position(|row| matches!(row.kind, HomeKind::Logo(_) | HomeKind::Heading("ARGOS OSINT")))
            .unwrap();
        assert!(logo_at <= 7, "top pad should be half the old even split, got {logo_at}");
        let headings: Vec<_> = rows
            .iter()
            .filter_map(|row| match row.kind {
                HomeKind::Heading(title) if !row.center => Some(title),
                _ => None,
            })
            .collect();
        assert_eq!(headings, ["Applications", "System"]);
        let apps: Vec<_> = rows
            .iter()
            .filter_map(|row| row.target.map(|index| ModuleId::ALL[index]))
            .take(3)
            .collect();
        assert_eq!(
            apps,
            [ModuleId::Intel, ModuleId::Atlas, ModuleId::Brain]
        );
        assert!(rows.iter().any(|row| {
            matches!(&row.kind, HomeKind::Logo(_)) && row.center
                || matches!(row.kind, HomeKind::Heading("ARGOS OSINT")) && row.center
        }));
    }

    #[test]
    fn atlas_source_anchors_are_not_labeled_deleted_origin() {
        let insight = recon::InsightView {
            memory_id: "m1".into(),
            entity: "delhi police".into(),
            predicate: "received".into(),
            object_value: "complaints".into(),
            topic: "crime".into(),
            classification: "fact".into(),
            confidence: 0.95,
            sources: vec![recon::InsightSource {
                thread_id: None,
                run_id: Some("atlas-1".into()),
                answer_id: "a1".into(),
                call_id: "art-1".into(),
                source_url: Some("https://example.com/story".into()),
                deleted_origin: false,
                published_at: String::new(),
            }],
            related: vec!["other".into()],
        };
        let text = memory_anchors_text(&insight);
        assert!(text.contains("delhi police"));
        assert!(text.contains("topic: crime"));
        assert!(text.contains("https://example.com/story"));
        assert!(!text.contains("deleted origin"));
        assert!(!text.contains("related"));
    }
}
