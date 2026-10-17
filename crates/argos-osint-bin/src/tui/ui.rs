//! Layout, chat transcript, and mouse hit areas for the Argos terminal shell.

use std::collections::{HashMap, HashSet};

use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;

use super::markdown::{self, Piece, Tone};
use super::recon_parts::InvestigationPart;

use super::app::{
    intel_category_short, intel_day_button_label, is_picker_field, unix_now, App, AtlasPage,
    BrainListMode, ButtonId, ChoiceKind, DefaultsRole, FieldId, IntelPage, LastViewSession,
    ModuleId, Overlay, ProviderPage, Target, INTEL_CATEGORIES,
};
use super::theme;
use argos_osint_core::atlas;
use argos_osint_core::intel_recon;
use argos_osint_core::osint;
use argos_osint_core::provider;
use argos_osint_core::recon::{self, Plan};

const TAB_H: u16 = 1;
const PAGE_TAB_H: u16 = 3;
pub(super) const FIELD_H: u16 = 2;
pub(super) const ACTION_H: u16 = 3;

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

pub(crate) struct Chrome {
    pub(crate) header: Rect,
    pub(crate) tab_strip: Rect,
    pub(crate) body: Rect,
    pub(crate) composer: Rect,
    pub(crate) footer: Rect,
    #[allow(dead_code)]
    pub(crate) home: Rect,
}

fn composer_height(area: Rect, app: &App) -> u16 {
    if app.module != Some(ModuleId::Recon) || !app.recon_chat {
        return 0;
    }
    let base_box_h = if area.height >= 26 { 3 } else { 2 };
    super::components::editor_height(&app.input, area.width.saturating_sub(4), base_box_h, 5)
}

/// Module body for the current screen (shared by dashboards for layout maths).
pub(crate) fn body_rect(app: &App) -> Rect {
    chrome(app.screen, app).body
}

/// Memory detail rectangles for the current screen and focus (draw, hit, scroll).
pub(crate) fn detail_areas(app: &App) -> super::brain_detail::DetailAreas {
    super::brain_detail::areas_for(app, body_rect(app))
}

fn chrome(area: Rect, app: &App) -> Chrome {
    let composer_h = composer_height(area, app);
    let header_h = TAB_H;
    let tab_strip_h = 0;
    let footer_h = if composer_h > 0 { 0 } else { 1 };
    let rows = split_vertical(
        area,
        [
            Constraint::Length(header_h),
            Constraint::Length(tab_strip_h),
            Constraint::Min(0),
            Constraint::Length(composer_h),
            Constraint::Length(footer_h),
        ],
    );
    let tabs = header_tabs(rows[0], app.module);
    let home = tabs.first().map(|(_, rect)| *rect).unwrap_or_default();
    Chrome {
        header: rows[0],
        tab_strip: rows[1],
        body: rows[2],
        composer: rows[3],
        footer: rows[4],
        home,
    }
}

fn header_tabs(area: Rect, active: Option<ModuleId>) -> Vec<(Option<ModuleId>, Rect)> {
    let mut labels = vec![(None, "[Home]".to_string())];
    if let Some(module) = active {
        labels.push((Some(module), module.title().to_string()));
    }
    let mut x = area.x;
    let mut out = Vec::new();
    for (module, label) in labels {
        let width = (label.len() as u16 + if module.is_none() { 3 } else { 2 })
            .min(area.width.saturating_sub(x - area.x));
        if width < 2 {
            break;
        }
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

pub(super) fn inset(area: Rect) -> Rect {
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

pub(super) fn button_areas(area: Rect, count: usize) -> Vec<Rect> {
    let count = count.max(1);
    split_horizontal(area, (0..count).map(|_| Constraint::Ratio(1, count as u32)))
}

fn composer_parts(area: Rect) -> (Rect, Rect) {
    let areas = recon_composer_areas(area);
    (areas.input, areas.send)
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
            Constraint::Length(FIELD_H),
            Constraint::Min(4),
            Constraint::Length(ACTION_H),
            Constraint::Length(8),
        ],
    );
    BrainList {
        query: rows[0],
        list: rows[1],
        actions: rows[2],
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

fn list_room(height: u16) -> usize {
    super::components::ListGeometry::capacity(height.saturating_sub(2) as usize, 1)
}

fn in_pane(rect: Rect, x: u16, y: u16) -> bool {
    x > rect.x
        && x + 1 < rect.x.saturating_add(rect.width)
        && y > rect.y
        && y + 1 < rect.y.saturating_add(rect.height)
}

fn model_areas(app: &App, area: Rect) -> Vec<Rect> {
    if area.width < 100 || area.height < 30 {
        let content = compact_content(area);
        if pane_page(app) == 0 {
            return vec![
                Rect::default(),
                Rect::default(),
                Rect::default(),
                Rect::default(),
                Rect::default(),
                Rect::default(),
                content,
            ];
        }
        let rows = split_vertical(
            content,
            [
                Constraint::Length(1),
                Constraint::Length(FIELD_H),
                Constraint::Length(FIELD_H),
                Constraint::Length(2),
                Constraint::Min(3),
                Constraint::Length(2),
                Constraint::Length(0),
            ],
        );
        return rows;
    }
    split_vertical(
        area,
        [
            Constraint::Length(ACTION_H),
            Constraint::Length(FIELD_H),
            Constraint::Length(FIELD_H),
            Constraint::Length(ACTION_H),
            Constraint::Min(4),
            Constraint::Length(ACTION_H),
            // Retry note (3) + role list/details incl. Brief operations: sized
            // to its content so the fallback list, not the details, absorbs
            // short terminals (fallbacks keep their 4-row minimum).
            Constraint::Length(MODEL_ROLES_H),
        ],
    )
}

/// Height of the role list/details pane (note + bordered details content).
const MODEL_ROLES_H: u16 = 23;

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

pub(crate) fn recon_workspace(app: &App, area: Rect) -> (Rect, Option<Rect>) {
    if !app.recon_context_enabled {
        return (area, None);
    }
    if app.screen.width < 110 {
        let content = Rect::new(
            area.x,
            area.y + area.height.min(1),
            area.width,
            area.height.saturating_sub(1),
        );
        return if app.compact_pages[ModuleId::Recon.index()] == 1 {
            (Rect::default(), Some(content))
        } else {
            (content, None)
        };
    }
    // 30% of the terminal viewport, then clipped to the remaining app body.
    let context_w = ((u32::from(app.screen.width) * 30) / 100)
        .min(u32::from(area.width.saturating_sub(1)))
        .max(1) as u16;
    if context_w == 0 || context_w >= area.width {
        return (area, None);
    }
    let transcript_w = area.width.saturating_sub(context_w);
    let panes = split_horizontal(
        area,
        [
            Constraint::Length(transcript_w),
            Constraint::Length(context_w),
        ],
    );
    (panes[0], Some(panes[1]))
}

fn recon_chat_areas(app: &App, body: Rect) -> (Rect, Option<Rect>, Option<Rect>) {
    let (transcript, context) = recon_workspace(app, body);
    if context.is_some() {
        (transcript, context, None)
    } else {
        let (workspace, run_actions) = chat_areas(body);
        (workspace, None, Some(run_actions))
    }
}

struct OsintLayout {
    search: Rect,
    list: Rect,
    detail: Rect,
    response: Rect,
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
    } else if id == "whoxy_whois_history" {
        (
            FieldId::WhoxyKey,
            FieldId::WhoxyFallback,
            ButtonId::SaveWhoxyKey,
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

pub(crate) fn pane_tabs(frame: &mut Frame, app: &App, area: Rect, labels: &[&str]) {
    let page = app
        .module
        .map_or(0, |module| app.compact_pages[module.index()]);
    for (index, rect) in button_areas(area, labels.len()).into_iter().enumerate() {
        app.layout
            .borrow_mut()
            .register(Target::PaneTab(index), rect);
        super::components::tab_button(
            frame,
            rect,
            labels[index],
            page == index,
            app.focus == Target::PaneTab(index),
        );
    }
}
fn pane_page(app: &App) -> usize {
    app.module
        .map_or(0, |module| app.compact_pages[module.index()])
}
fn compact_content(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y + 1,
        area.width,
        area.height.saturating_sub(1),
    )
}

fn osint_areas(app: &App, area: Rect, with_key: bool) -> OsintLayout {
    let top = split_vertical(area, [Constraint::Length(FIELD_H), Constraint::Min(0)]);
    let compact = area.width < 68 || area.height < 26;
    let columns = if compact {
        let content = compact_content(top[1]);
        if pane_page(app) == 0 {
            vec![content, Rect::default()]
        } else {
            vec![Rect::default(), content]
        }
    } else {
        split_horizontal(
            top[1],
            [Constraint::Percentage(38), Constraint::Percentage(62)],
        )
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
    let (mut detail, mut response) = if compact {
        if pane_page(app) == 1 {
            (columns[1], Rect::default())
        } else {
            (Rect::default(), right[0])
        }
    } else {
        let sections = split_vertical(
            right[0],
            [Constraint::Percentage(50), Constraint::Percentage(50)],
        );
        (sections[0], sections[1])
    };
    if compact && pane_page(app) == 0 {
        detail = Rect::default();
        response = Rect::default();
    }
    let show_form = !compact || pane_page(app) == 2;
    OsintLayout {
        search: top[0],
        list: columns[0],
        detail,
        response,
        key: if with_key && show_form {
            right[1]
        } else {
            Rect::default()
        },
        fallback: if with_key && show_form {
            right[2]
        } else {
            Rect::default()
        },
        input: if show_form {
            right[if with_key { 3 } else { 1 }]
        } else {
            Rect::default()
        },
        actions: if show_form {
            right[if with_key { 4 } else { 2 }]
        } else {
            Rect::default()
        },
    }
}

/// System: the Refresh hardware action row, then host and path panes.
pub(crate) fn system_areas(area: Rect) -> (Rect, Rect) {
    let rows = split_vertical(area, [Constraint::Length(ACTION_H), Constraint::Min(0)]);
    (rows[1], rows[0])
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

/// The Configs popup covers about 85% of the viewport, bounded by the terminal:
/// large enough for the import editor, never larger than the screen.
pub(crate) fn configs_area(app: &App, area: Rect) -> Rect {
    if matches!(&app.overlay, Overlay::Configs) {
        let width = (area.width as f32 * 0.85) as u16;
        let height = (area.height as f32 * 0.85) as u16;
        let width = width.clamp(40, 140).min(area.width);
        let height = height.clamp(12, 48).min(area.height);
        Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        }
    } else {
        area
    }
}

pub(crate) fn active_popup_area(app: &App, area: Rect) -> Rect {
    if matches!(&app.overlay, Overlay::ResumeSession(_)) {
        let width = 64.min(area.width.saturating_sub(4));
        let height = 12.min(area.height.saturating_sub(2));
        Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        }
    } else if matches!(&app.overlay, Overlay::Block { title, .. } if title.to_ascii_lowercase().contains("report"))
    {
        let width = ((u32::from(area.width) * 70) / 100).max(1) as u16;
        let height = ((u32::from(area.height) * 80) / 100).max(1) as u16;
        Rect {
            x: area.x + area.width.saturating_sub(width) / 2,
            y: area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        }
    } else {
        popup_area(area)
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

pub(crate) struct HomeLayoutMetrics {
    pub(crate) top_pad: u16,
    pub(crate) box_y: u16,
    pub(crate) box_h: u16,
    pub(crate) guidance_y: u16,
    pub(crate) apps_y: u16,
    pub(crate) composer_width: u16,
    pub(crate) composer_x: u16,
}

pub(crate) fn home_layout_metrics(area: Rect) -> HomeLayoutMetrics {
    if area.height < 18 {
        let top_pad = area.height.saturating_sub(8) / 2;
        return HomeLayoutMetrics {
            top_pad,
            box_y: area.y + top_pad,
            box_h: 2,
            guidance_y: area.y + top_pad + 2,
            apps_y: area.y + top_pad + 3,
            composer_width: area.width,
            composer_x: area.x,
        };
    }
    let is_wide = area.width as usize >= logo_width() && area.height >= 38;
    let title_count = if is_wide { 6 } else { 1 };
    let composer_width = if is_wide {
        (logo_width() as u16).min(area.width.saturating_sub(4))
    } else {
        area.width.saturating_sub(4).max(40).min(area.width)
    };
    let composer_x = area.x + (area.width.saturating_sub(composer_width)) / 2;

    let box_h = if area.height >= 26 { 3 } else { 2 };
    let composer_total_h = box_h + 1; // box_h + 1 guidance (label removed)
    let apps_h = if area.height < 32 { 5 } else { 12 }; // dense app rows; retain group headings and section gap

    let (gap_title_to_composer, gap_composer_to_apps) =
        if area.height >= 34 { (2, 2) } else { (1, 1) };

    let content_h =
        title_count + gap_title_to_composer + composer_total_h + gap_composer_to_apps + apps_h;
    let top_pad = area.height.saturating_sub(content_h) / 2;

    let title_y = area.y.saturating_add(top_pad);
    let title_end_y = title_y.saturating_add(title_count);
    let box_y = title_end_y.saturating_add(gap_title_to_composer);
    let guidance_y = box_y.saturating_add(box_h);
    let apps_y = guidance_y
        .saturating_add(1)
        .saturating_add(gap_composer_to_apps);

    HomeLayoutMetrics {
        top_pad,
        box_y,
        box_h,
        guidance_y,
        apps_y,
        composer_width,
        composer_x,
    }
}

pub(crate) fn home_rows(area: Rect, errors: usize) -> Vec<HomeRow> {
    let metrics = home_layout_metrics(area);
    let is_wide = area.width as usize >= logo_width() && area.height >= 38;

    let mut rows = Vec::new();
    for _ in 0..metrics.top_pad {
        rows.push(gap_row());
    }

    if is_wide {
        rows.extend(logo_rows());
    } else {
        rows.push(center_row(HomeKind::Heading("ARGOS OSINT")));
    }

    let mut app_rows = Vec::new();
    home_group(
        &mut app_rows,
        "Applications",
        &[
            ModuleId::Intel,
            ModuleId::Atlas,
            ModuleId::Brain,
            ModuleId::Recon,
        ],
        errors,
    );
    app_rows.push(gap_row());
    home_group(
        &mut app_rows,
        "System",
        &[
            ModuleId::Jobs,
            ModuleId::Logs,
            ModuleId::Osint,
            ModuleId::Providers,
            ModuleId::System,
        ],
        errors,
    );

    for (index, row) in rows.iter_mut().enumerate() {
        row.y = area.y.saturating_add(index as u16);
    }

    if area.height < 32 {
        app_rows.retain(|row| row.target.is_some());
    }
    for (index, row) in app_rows.iter_mut().enumerate() {
        row.y = metrics.apps_y.saturating_add(if area.height < 32 {
            (index / 2) as u16
        } else {
            index as u16
        });
    }

    rows.extend(app_rows);

    let max_y = area.y.saturating_add(area.height);
    rows.retain(|r| r.y < max_y);
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
    if module == ModuleId::Logs && errors > 0 {
        format!(
            "{} · {}",
            module.blurb(),
            super::logs::count(errors as i64, "error")
        )
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

#[derive(Clone, Debug)]
pub struct ChatBlock {
    pub part: InvestigationPart,
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
    let runs_by_turn: HashMap<&str, &recon::Run> = app
        .runs
        .iter()
        .map(|run| (run.turn_id.as_str(), run))
        .collect();
    let runs_by_id: HashMap<&str, &recon::Run> =
        app.runs.iter().map(|run| (run.id.as_str(), run)).collect();
    let answered: HashSet<&str> = app
        .messages
        .iter()
        .filter(|message| message.role == "assistant")
        .filter_map(|message| message.run_id.as_deref())
        .collect();
    let mut calls_by_run: HashMap<&str, Vec<(usize, &recon::Call)>> = HashMap::new();
    for (index, call) in app.calls.iter().enumerate() {
        if let Some(run_id) = call.run_id.as_deref() {
            calls_by_run.entry(run_id).or_default().push((index, call));
        }
    }
    // One live answer per open thread. It is appended after every tool row so a plan-log
    // refresh or a late call cannot push the text the user is reading off the bottom.
    let mut live: Option<ChatBlock> = None;
    for (index, message) in app.messages.iter().enumerate() {
        if message.role == "user" {
            blocks.push(ChatBlock {
                part: InvestigationPart::UserQuery,
                key: format!("user:{}", message.id),
                title: "You".into(),
                body: message.content.clone(),
                collapsible: false,
                message_index: Some(index),
                has_memory: false,
            });
            if let Some(run) = runs_by_turn.get(message.id.as_str()).copied() {
                blocks.push(plan_block(
                    run,
                    &app.calls,
                    app.expanded.contains(&format!("plan:{}", run.id)),
                ));
                if !app.expanded.contains(&format!("plan:{}", run.id)) {
                    if let Some(error) = run.error.as_deref().filter(|error| !error.is_empty()) {
                        blocks.push(ChatBlock {
                            part: InvestigationPart::Status,
                            key: format!("status:error:{}", run.id),
                            title: format!("! Plan error · {}", clip_chars(error, 160)),
                            body: String::new(),
                            collapsible: false,
                            message_index: None,
                            has_memory: false,
                        });
                    }
                }
                if app.expanded.contains(&format!("plan:{}", run.id)) {
                    if let Some(plan) = run
                        .plan_json
                        .as_deref()
                        .and_then(|raw| serde_json::from_str::<Plan>(raw).ok())
                    {
                        if !plan.directives.is_empty() {
                            let key = format!("plan-details:{}", run.id);
                            let body = if app.expanded.contains(&key) {
                                question_plan_lines(run, &plan, &app.calls).join("\n")
                            } else {
                                String::new()
                            };
                            blocks.push(ChatBlock {
                                part: InvestigationPart::PlanDiagnostics,
                                key,
                                title: "Plan details · picker, bindings, and execution notes"
                                    .into(),
                                body,
                                collapsible: true,
                                message_index: None,
                                has_memory: false,
                            });
                        }
                    }
                }
                if let Some(calls) = calls_by_run.get(run.id.as_str()) {
                    let mut grouped = HashSet::new();
                    for &(call_index, call) in calls {
                        used.insert(call_index);
                        if matches!(call.status.as_str(), "completed" | "no_results") {
                            let peers: Vec<_> = calls
                                .iter()
                                .filter(|(_, peer)| {
                                    peer.tool_id == call.tool_id
                                        && matches!(
                                            peer.status.as_str(),
                                            "completed" | "no_results"
                                        )
                                })
                                .collect();
                            if peers.len() > 1 {
                                let key = format!("activity-group:{}:{}", run.id, call.tool_id);
                                if grouped.insert(call.tool_id.as_str()) {
                                    blocks.push(ChatBlock {
                                        part: InvestigationPart::ToolActivity,
                                        key: key.clone(),
                                        title: format!("✓ {} · {} completed evidence activities · Enter details", call.tool_id, peers.len()),
                                        body: String::new(), collapsible: true, message_index: None, has_memory: false,
                                    });
                                }
                                if !app.expanded.contains(&key)
                                    && !app.expanded.contains(&format!("tool:{}", call.id))
                                {
                                    continue;
                                }
                            }
                        }
                        blocks.push(tool_block(app, call, call_index));
                    }
                }
                let has_answer = answered.contains(run.id.as_str());
                if !has_answer && app.running_thread(&run.thread_id) {
                    let stage = app.stage_label(&run.thread_id);
                    let deadline = app.deadline_label(&run.thread_id);
                    let title = if deadline.is_empty() {
                        format!("· {stage}")
                    } else {
                        format!("· {stage} · {deadline}")
                    };
                    blocks.push(ChatBlock {
                        part: InvestigationPart::Status,
                        key: format!("status:{}", run.id),
                        title,
                        body: String::new(),
                        collapsible: false,
                        message_index: None,
                        has_memory: false,
                    });
                }
                if !has_answer {
                    if let Some((title, body)) = app.live_bubble(&run.thread_id) {
                        live = Some(ChatBlock {
                            part: InvestigationPart::StreamingSynthesis,
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
                part: InvestigationPart::Synthesis,
                key: format!("assistant:{}", message.id),
                title: "Recon".into(),
                body: message.content.clone(),
                collapsible: false,
                message_index: Some(index),
                has_memory: memories,
            });
            if let Some(run) = message
                .run_id
                .as_deref()
                .and_then(|id| runs_by_id.get(id).copied())
            {
                if let Some(plan) = run
                    .plan_json
                    .as_deref()
                    .and_then(|raw| serde_json::from_str::<Plan>(raw).ok())
                {
                    if !plan.directives.is_empty() {
                        blocks.push(ChatBlock {
                            part: InvestigationPart::DirectiveAssessment,
                            key: format!("status:coverage:{}", message.id),
                            title: coverage_summary(&plan, &message.content),
                            body: String::new(),
                            collapsible: false,
                            message_index: None,
                            has_memory: false,
                        });
                    }
                }
                let memory_count = app.answer_memories.get(&message.id).map_or(0, Vec::len);
                let call_count = calls_by_run.get(run.id.as_str()).map_or(0, Vec::len);
                blocks.push(ChatBlock {
                    part: InvestigationPart::TurnSummary,
                    key: format!("status:summary:{}", message.id),
                    title: format!("Investigation · {} · {call_count} evidence calls · {memory_count} linked memories", run.state),
                    body: String::new(),
                    collapsible: false,
                    message_index: None,
                    has_memory: false,
                });
            }
        }
    }
    for (call_index, call) in app.calls.iter().enumerate() {
        if used.insert(call_index) {
            blocks.push(tool_block(app, call, call_index));
        }
    }
    for event in &app.investigation_events {
        blocks.push(super::investigation_trace::event_to_chat_block(event, None));
    }
    if let Some(block) = live {
        blocks.push(block);
    }
    blocks
}

fn coverage_summary(plan: &Plan, answer: &str) -> String {
    let mut met = 0;
    let mut partial = 0;
    let mut not_met = 0;
    let mut unassessed = Vec::new();
    for directive in &plan.directives {
        let prefix = format!("{}:", directive.id.to_ascii_lowercase());
        let assessment = answer.lines().find_map(|line| {
            let line = line.trim().to_ascii_lowercase();
            line.strip_prefix(&prefix)
                .map(|tail| tail.trim().to_string())
        });
        match assessment.as_deref() {
            Some(text) if text.starts_with("partly met") || text.starts_with("partially met") => {
                partial += 1
            }
            Some(text) if text.starts_with("not met") => not_met += 1,
            Some(text) if text.starts_with("met") => met += 1,
            _ => unassessed.push(directive.id.to_uppercase()),
        }
    }
    let mut parts = vec![format!(
        "Coverage · {met} met · {partial} partial · {not_met} not met"
    )];
    if !unassessed.is_empty() {
        parts.push(format!("{} unassessed", unassessed.join(", ")));
    }
    parts.join(" · ")
}

fn plan_block(run: &recon::Run, calls: &[recon::Call], open: bool) -> ChatBlock {
    let plan = run
        .plan_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Plan>(raw).ok());
    let title = match &plan {
        Some(plan) => {
            let mut parts = vec![format!(
                "{} Plan",
                if plan.planning_mode == "tool_picker_fallback" {
                    "!"
                } else {
                    "◆"
                }
            )];
            if plan.planning_mode == "tool_picker_fallback" {
                parts.push("deterministic fallback".into());
            }
            if !plan.directives.is_empty() {
                parts.push(format!("{} directives", plan.directives.len()));
            }
            parts.push(format!("{} tools", plan.calls.len()));
            if let Some(mode) = argos_osint_core::intel_recon::ReportMode::parse(&plan.report_mode)
            {
                parts.push(mode.title().into());
            }
            parts.join(" · ")
        }
        None => "◌ Plan · selecting tools".into(),
    };
    let body = if !open {
        String::new()
    } else {
        match plan {
            Some(plan) if !plan.directives.is_empty() => {
                plan_summary_lines(run, &plan, calls).join("\n")
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
        part: InvestigationPart::Plan,
        key: format!("plan:{}", run.id),
        title,
        body,
        collapsible: true,
        message_index: None,
        has_memory: false,
    }
}

/// Analyst-facing plan content. Execution state stays separate from directive support.
fn plan_summary_lines(run: &recon::Run, plan: &Plan, calls: &[recon::Call]) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push("Directives · evidence assessment pending".into());
    for directive in &plan.directives {
        if lines.last().is_some_and(|line: &String| !line.is_empty()) {
            lines.push(String::new());
        }
        lines.push(format!(
            "  {}  {}  ○ unassessed",
            directive.id.to_uppercase(),
            directive.goal
        ));
    }
    if !plan.calls.is_empty() {
        lines.push(String::new());
        lines.push("Collection strategy".into());
    }
    for (index, step) in plan.calls.iter().enumerate() {
        if index > 0 {
            lines.push(String::new());
        }
        let name = osint::definition(&step.tool_id)
            .map(|tool| tool.name)
            .unwrap_or(step.tool_id.as_str());
        let state = matching_call(calls, step)
            .map(|call| call.status.as_str())
            .unwrap_or(step.status.as_str());
        let state = if state.is_empty() { "pending" } else { state };
        let serves = if step.reason.is_empty() {
            String::new()
        } else {
            format!(" · {}", step.reason)
        };
        let depends = if step.depends_on.is_empty() {
            String::new()
        } else {
            format!(" · after {}", step.depends_on.join(", "))
        };
        lines.push(format!(
            "  {}  {name}{serves}{depends} · {state}",
            index + 1
        ));
    }
    if !plan.fallback_requests.is_empty() {
        lines.push(String::new());
        lines.push("Fallbacks".into());
        for fallback in &plan.fallback_requests {
            if lines.last().is_some_and(|line: &String| !line.is_empty()) {
                lines.push(String::new());
            }
            lines.push(format!("  · {}", clip_chars(fallback, 100)));
        }
    }
    if let Some(error) = run.error.as_deref().filter(|error| !error.is_empty()) {
        lines.push(format!("Run error: {}", clip_chars(error, 160)));
    }
    lines
}

/// Picker, binding, and input details exposed by the secondary disclosure.
fn question_plan_lines(run: &recon::Run, plan: &Plan, calls: &[recon::Call]) -> Vec<String> {
    let mut lines = Vec::new();
    if !plan.report_mode.is_empty() {
        let title = argos_osint_core::intel_recon::ReportMode::parse(&plan.report_mode)
            .map(|mode| mode.title())
            .unwrap_or(plan.report_mode.as_str());
        lines.push(format!("Report mode: {title}"));
    }
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
        if lines.last().is_some_and(|line: &String| !line.is_empty()) {
            lines.push(String::new());
        }
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
        if lines.last().is_some_and(|line: &String| !line.is_empty()) {
            lines.push(String::new());
        }
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
    let format_seed = |binding: &recon::Binding| {
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
    };
    let from_brain: Vec<_> = plan
        .bindings
        .iter()
        .filter(|binding| {
            binding.step_id.is_empty()
                && !binding.inferred
                && binding.evidence_id.starts_with("brain:")
        })
        .map(format_seed)
        .collect();
    if !from_brain.is_empty() {
        lines.push(format!("From Brain: {}", from_brain.join("; ")));
    }
    let from_question: Vec<_> = plan
        .bindings
        .iter()
        .filter(|binding| {
            binding.step_id.is_empty()
                && !binding.inferred
                && !binding.evidence_id.starts_with("brain:")
        })
        .map(format_seed)
        .collect();
    if !from_question.is_empty() {
        lines.push(format!("From the question: {}", from_question.join("; ")));
    }
    if !plan.binding_notes.is_empty() {
        lines.push("Binding extraction:".into());
        for note in &plan.binding_notes {
            lines.push(format!("   {note}"));
        }
    }
    if !plan.picks.is_empty() {
        lines.push("Picker decisions:".into());
        for pick in &plan.picks {
            let confidence = pick
                .confidence
                .map(|value| format!(" · p={value:.2}"))
                .unwrap_or_default();
            lines.push(format!(
                "   {}. {} · {}{confidence}",
                pick.position, pick.tool_id, pick.outcome
            ));
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

/// One-line summary and the clipped result body for a Logs entry.
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

fn tool_block(app: &App, call: &recon::Call, call_index: usize) -> ChatBlock {
    let name = osint::definition(&call.tool_id)
        .map(|tool| tool.name)
        .unwrap_or(call.tool_id.as_str());
    let key = format!("tool:{}", call.id);
    let state = match call.status.as_str() {
        "completed" => "✓ complete",
        "no_results" => "○ no results",
        "failed" | "timeout" => "✗ failed",
        "cancelled" => "− cancelled",
        _ => "◌ running",
    };
    let count = call
        .result
        .as_ref()
        .and_then(|result| result_count(&result.observations))
        .map(|count| format!(" · {count} results"))
        .unwrap_or_default();
    let mut title = format!("{state} · E{} · {name}{count}", call_index + 1);
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
    if let Some(duration) = call.completed_at.as_deref().and_then(|end| {
        let end = chrono::DateTime::parse_from_rfc3339(end).ok()?;
        let start = chrono::DateTime::parse_from_rfc3339(&call.started_at).ok()?;
        Some((end - start).num_milliseconds().max(0))
    }) {
        title.push_str(&format!(" · {duration}ms"));
    }
    title.push_str(&inputs);
    title.push_str(cache);
    if call.attempts > 1 {
        title.push_str(&format!(" · {} attempts", call.attempts));
    }
    if let Some(error) = call
        .result
        .as_ref()
        .and_then(|result| result.error.as_deref())
    {
        title.push_str(&format!(" · {}", super::components::clip_text(error, 80)));
    }
    if !app.expanded.contains(&key) {
        return ChatBlock {
            part: InvestigationPart::ToolActivity,
            key,
            title,
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
    lines.push("Full result is in Logs.".into());
    ChatBlock {
        part: InvestigationPart::ToolActivity,
        key,
        title: format!("{title}{cache}{inputs}"),
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
    super::components::clip_text(value, max)
}
fn wrapped_line_count(text: &str, width: usize) -> usize {
    super::components::measured_lines(
        vec![Line::raw(text.to_owned())],
        width.min(u16::MAX as usize) as u16,
    )
    .len()
}

pub(crate) fn center_line(value: &str, width: usize) -> String {
    center_text(value, width)
}

fn center_text(value: &str, width: usize) -> String {
    let shown = fit(value, width);
    let pad = width.saturating_sub(super::components::text_width(&shown)) / 2;
    format!("{:pad$}{shown}", "", pad = pad)
}

pub(super) fn fit(value: &str, width: usize) -> String {
    super::components::clip_text(value, width)
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
    let heading = if label.starts_with("! Plan") {
        Tone::Warn
    } else if label.starts_with('✓') {
        Tone::Success
    } else if label.starts_with('✗') {
        Tone::Error
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
    live_shown: u64,
    live_note: String,
    deadline: String,
    messages: u64,
    calls: u64,
    runs: u64,
    events: u64,
    expanded: Vec<String>,
}

#[derive(Default)]
pub struct FrameCache {
    stamp: FrameStamp,
    blocks: Vec<ChatBlock>,
    rows: Vec<ChatRow>,
    frozen: HashMap<String, super::components::TranscriptBlock<Vec<markdown::MdLine>>>,
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
            .map(|(_, body)| text_revision(&body))
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
        events: text_revision(&format!("{:?}", app.investigation_events)),
        expanded,
    }
}

fn mix(acc: u64, value: u64) -> u64 {
    acc.wrapping_mul(0x9E37_79B1_85EB_CA87).wrapping_add(value)
}

fn text_revision(text: &str) -> u64 {
    text.bytes().fold(0, mix_byte)
}
fn mix_byte(hash: u64, byte: u8) -> u64 {
    mix(hash, u64::from(byte))
}
fn message_stamp(messages: &[recon::Message]) -> u64 {
    let mut acc = messages.len() as u64;
    for message in messages {
        acc = mix(acc, text_revision(&message.content));
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
            .map(|result| mix(text_revision(&result.raw), text_revision(&result.status)))
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
                .map(|plan| text_revision(plan))
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
    let mut frozen = std::mem::take(&mut app.frame.borrow_mut().frozen);
    frozen.retain(|key, _| blocks.iter().any(|block| &block.key == key));
    let rows = rows_for(app, &blocks, width as usize, &mut frozen);
    *app.frame.borrow_mut() = FrameCache {
        stamp,
        blocks,
        rows,
        frozen,
    };
}

fn expanded(app: &App, block: &ChatBlock) -> bool {
    if !block.collapsible {
        return true;
    }
    if block.part == InvestigationPart::Thinking {
        return app.show_thinking || app.expanded.contains(&block.key);
    }
    app.expanded.contains(&block.key)
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

fn streaming_boundary(text: &str) -> usize {
    let mut boundary = 0;
    let mut position = 0;
    let mut fence = false;
    for line in text.split_inclusive('\n') {
        position += line.len();
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        if !fence && line.trim().is_empty() {
            boundary = position;
        }
    }
    boundary
}

fn rows_for(
    app: &App,
    blocks: &[ChatBlock],
    width: usize,
    frozen: &mut HashMap<String, super::components::TranscriptBlock<Vec<markdown::MdLine>>>,
) -> Vec<ChatRow> {
    let width = width.max(1);
    let mut rows = Vec::new();
    for (index, block) in blocks.iter().enumerate() {
        if index > 0
            && rows
                .last()
                .is_some_and(|row: &ChatRow| !row.pieces.is_empty())
        {
            rows.push(row(index, false, false, false, RowFace::Plain, Vec::new()));
        }
        if block.part == InvestigationPart::UserQuery {
            for (line_index, pieces) in markdown::user_lines(&block.body, width.saturating_sub(3))
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
        if block.part == InvestigationPart::Synthesis {
            let revision = text_revision(&block.body);
            let cached = frozen.entry(block.key.clone()).or_insert_with(|| {
                super::components::TranscriptBlock {
                    revision,
                    width,
                    content: markdown::markdown_lines(&block.body, width),
                }
            });
            if cached.revision != revision || cached.width != width {
                *cached = super::components::TranscriptBlock {
                    revision,
                    width,
                    content: markdown::markdown_lines(&block.body, width),
                };
            }
            let lines = cached.content.clone();
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
        if block.part == InvestigationPart::Thinking {
            let open = expanded(app, block);
            let indicator = if open { "▼ " } else { "▶ " };
            let mut pieces = vec![Piece {
                text: format!("{indicator}{}", block.title),
                tone: Tone::Dim,
            }];
            clip_pieces(&mut pieces, width);
            rows.push(row(index, true, false, true, RowFace::Plain, pieces));
            if open && !block.body.is_empty() {
                for pieces in markdown::plain_lines(&block.body, width, 2) {
                    rows.push(row(index, false, false, false, RowFace::Plain, pieces));
                }
            }
            continue;
        }
        if matches!(
            block.part,
            InvestigationPart::RoleDecision
                | InvestigationPart::GateValidation
                | InvestigationPart::Handoff
                | InvestigationPart::EvidencePassage
        ) {
            let open = expanded(app, block);
            let indicator = if open { "▼ " } else { "▶ " };
            let mut pieces = vec![Piece {
                text: format!("{indicator}{}", block.title),
                tone: Tone::Dim,
            }];
            clip_pieces(&mut pieces, width);
            rows.push(row(index, true, false, true, RowFace::Plain, pieces));
            if open && !block.body.is_empty() {
                for pieces in markdown::plain_lines(&block.body, width, 2) {
                    rows.push(row(index, false, false, false, RowFace::Plain, pieces));
                }
            }
            continue;
        }
        if matches!(
            block.part,
            InvestigationPart::Status
                | InvestigationPart::DirectiveAssessment
                | InvestigationPart::TurnSummary
        ) {
            let mut pieces = vec![Piece {
                text: block.title.clone(),
                tone: Tone::Dim,
            }];
            clip_pieces(&mut pieces, width);
            rows.push(row(index, true, false, false, RowFace::Plain, pieces));
            continue;
        }
        if block.part == InvestigationPart::StreamingSynthesis {
            let mut pieces = vec![Piece {
                text: block.title.clone(),
                tone: Tone::Accent,
            }];
            clip_pieces(&mut pieces, width);
            rows.push(row(index, true, false, false, RowFace::Plain, pieces));
            let boundary = streaming_boundary(&block.body);
            let prefix = &block.body[..boundary];
            let revision = text_revision(prefix);
            let cached = frozen.entry(block.key.clone()).or_insert_with(|| {
                super::components::TranscriptBlock {
                    revision,
                    width,
                    content: if prefix.is_empty() {
                        Vec::new()
                    } else {
                        markdown::markdown_lines(prefix, width)
                    },
                }
            });
            if cached.revision != revision || cached.width != width {
                *cached = super::components::TranscriptBlock {
                    revision,
                    width,
                    content: if prefix.is_empty() {
                        Vec::new()
                    } else {
                        markdown::markdown_lines(prefix, width)
                    },
                };
            }
            let tail = markdown::markdown_lines(&block.body[boundary..], width);
            for line in cached.content.iter().cloned().chain(tail) {
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
    inset(recon_chat_areas(app, body).0)
}

fn chat_view(app: &App) -> (Rect, usize, Vec<ChatRow>) {
    ensure_frame(app);
    let inner = transcript_rect(app);
    let rows = app.frame.borrow().rows.clone();
    let max = rows.len().saturating_sub(inner.height as usize);
    let scroll = if app.chat_follow {
        max
    } else {
        app.scrolls.chat.min(max)
    };
    (inner, scroll, rows)
}

fn chat_max(app: &App) -> usize {
    let (inner, _, rows) = chat_view(app);
    rows.len().saturating_sub(inner.height as usize)
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
    for (offset, (_index, row)) in rows.iter().enumerate().skip(scroll).enumerate() {
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
        } else if !row.pieces.is_empty() {
            spots.push(Spot {
                rect,
                target: Target::ChatBody(row.block),
            });
        }
    }
    spots
}

pub(crate) fn reveal_spaced_lists(app: &mut App) {
    let body = chrome(app.screen, app).body;
    if app.module.is_none() && body.height < 32 {
        let metrics = home_layout_metrics(body);
        let room = super::components::ListGeometry::capacity_with_gap(
            body.bottom().saturating_sub(metrics.apps_y) as usize,
            1,
            0,
        )
        .max(1);
        reveal_index(&mut app.scrolls.home, app.launcher_sel / 2, room);
    }
    if app.module == Some(ModuleId::Providers)
        && app.provider_page == ProviderPage::Defaults
        && app.overlay == Overlay::None
    {
        let models = model_areas(app, provider_areas(body)[1]);
        let room = super::components::ListGeometry::capacity(
            models[6].height.saturating_sub(2) as usize,
            1,
        )
        .max(1);
        let reveal_role = match app.focus {
            Target::Button(ButtonId::DefaultRole(role)) => role,
            _ => app.defaults_role,
        };
        let selected = DefaultsRole::ALL
            .iter()
            .position(|role| *role == reveal_role)
            .unwrap_or(0);
        reveal_index(&mut app.scrolls.roles, selected, room);
        let room =
            super::components::ListGeometry::capacity(inset(models[4]).height as usize, 1).max(1);
        reveal_index(&mut app.scrolls.fallbacks, app.fallback_sel, room);
    }
    if app.overlay == Overlay::AddFallback {
        let rows = add_fallback_layout(active_popup_area(app, app.screen));
        let room =
            super::components::ListGeometry::capacity(inset(rows[3]).height as usize, 1).max(1);
        reveal_index(&mut app.scrolls.popup, app.fallback_popup_sel, room);
    }
}

pub fn normalize(app: &mut App) {
    reveal_spaced_lists(app);
    let anchor = if !app.chat_follow {
        let old = app.frame.borrow();
        if old.stamp.thread == app.selected_thread {
            old.rows.get(app.scrolls.chat).and_then(|row| {
                old.blocks.get(row.block).map(|block| {
                    let start = old
                        .rows
                        .iter()
                        .position(|candidate| candidate.block == row.block)
                        .unwrap_or(0);
                    (block.key.clone(), app.scrolls.chat.saturating_sub(start))
                })
            })
        } else {
            None
        }
    } else {
        None
    };
    let count = chat_blocks(app).len();
    if count == 0 {
        app.chat_sel = 0;
    } else if app.chat_follow || app.chat_sel >= count {
        app.chat_sel = count - 1;
    }
    if let Some((key, local)) = anchor {
        let new = app.frame.borrow();
        if let Some(block) = new.blocks.iter().position(|block| block.key == key) {
            if let Some(start) = new.rows.iter().position(|row| row.block == block) {
                let length = new
                    .rows
                    .iter()
                    .skip(start)
                    .take_while(|row| row.block == block)
                    .count();
                app.scrolls.chat = start + local.min(length.saturating_sub(1));
            }
        }
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
    let start = app.scrolls.chat;
    if line < start {
        app.scrolls.chat = line;
    } else if line >= start + room {
        app.scrolls.chat = line + 1 - room;
    }
}

pub fn navigate_turn(app: &mut App, next: bool) {
    let blocks = chat_blocks(app);
    let turns: Vec<_> = blocks
        .iter()
        .enumerate()
        .filter(|(_, block)| block.part == InvestigationPart::UserQuery)
        .map(|(index, _)| index)
        .collect();
    let target = if next {
        turns.into_iter().find(|index| *index > app.chat_sel)
    } else {
        turns.into_iter().rev().find(|index| *index < app.chat_sel)
    };
    if let Some(index) = target {
        app.chat_sel = index;
        app.chat_follow = false;
        reveal_chat(app);
        app.set_focus(Target::ChatHeader(index));
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
    if app.overlay == Overlay::AddFallback {
        let last = app.filtered_fallback_models().len().saturating_sub(1);
        app.fallback_popup_sel = app
            .fallback_popup_sel
            .saturating_add_signed(delta as isize)
            .min(last);
        reveal_spaced_lists(app);
        return;
    }
    if app.overlay != Overlay::None {
        let max = popup_max(app);
        nudge(&mut app.scrolls.popup, delta * 3, max);
        return;
    }
    if app.module == Some(ModuleId::Brain) && app.brain_list_mode == BrainListMode::Topology {
        let target = hit_test(app, x, y).unwrap_or(app.focus);
        super::topology::scroll_topology(app, target, delta as isize * 3);
        return;
    }
    let body = chrome(app.screen, app).body;
    if app.module.is_none() {
        let index = app
            .launcher_sel
            .saturating_add_signed(delta as isize * 2)
            .min(ModuleId::ALL.len() - 1);
        app.set_focus(Target::App(index));
        return;
    }
    if app.module == Some(ModuleId::Providers) && app.provider_page == ProviderPage::Defaults {
        let models = model_areas(app, provider_areas(body)[1]);
        if contains(models[4], x, y) {
            let count = app
                .settings
                .defaults
                .role(app.defaults_role.role_key())
                .map_or(0, |role| role.fallbacks.len());
            let index = app
                .fallback_sel
                .saturating_add_signed(delta as isize)
                .min(count.saturating_sub(1));
            app.set_focus(Target::Button(ButtonId::FallbackItem(index)));
            return;
        }
        if contains(models[6], x, y) {
            let focused_role = match app.focus {
                Target::Button(ButtonId::DefaultRole(role)) => role,
                _ => app.defaults_role,
            };
            let index = DefaultsRole::ALL
                .iter()
                .position(|role| *role == focused_role)
                .unwrap_or(0);
            let next = index
                .saturating_add_signed(delta as isize)
                .min(DefaultsRole::ALL.len() - 1);
            app.set_focus(Target::Button(ButtonId::DefaultRole(
                DefaultsRole::ALL[next],
            )));
            return;
        }
    }
    if app.module == Some(ModuleId::System) {
        if app.profile.filter_popup.is_none() && !app.profile.period_popup {
            if let Some(
                Target::ProfileCard(index)
                | Target::ProfilePanelBucket(index, _)
                | Target::ProfilePanelRow(index, _),
            ) = hit_test(app, x, y)
            {
                if app.focus != Target::ProfileCard(index) {
                    app.set_focus(Target::ProfileCard(index));
                }
            }
            if let Some(Target::ProfileTable(index) | Target::ProfileTableRow(index, _)) =
                hit_test(app, x, y)
            {
                app.set_focus(Target::ProfileTable(index));
            }
            super::profile::scroll(app, delta as isize * 3);
        }
        return;
    }
    if app.intel_page == IntelPage::Bulletin && hit_test(app, x, y) == Some(Target::IntelPreview) {
        app.intel_preview_scroll = app
            .intel_preview_scroll
            .saturating_add_signed((delta * 3) as isize);
        return;
    }
    if app.module == Some(ModuleId::Osint) {
        let layout = osint_areas(app, body, api_key_slot(app).is_some());
        if contains(layout.response, x, y) {
            app.response_scroll = app
                .response_scroll
                .saturating_add_signed((delta * 3) as isize);
            return;
        }
    }
    if app.module == Some(ModuleId::Logs) {
        if super::logs::in_list(body, x, y) {
            let (room, width) = super::logs::list_geometry(body);
            let max = app.logs.scroll_max(room, width);
            nudge(&mut app.logs.scroll, delta * 3, max);
        }
        return;
    }
    if app.module == Some(ModuleId::Jobs) {
        let areas = super::jobs::areas(body, &app.jobs);
        if contains(areas.detail, x, y) {
            let max = super::jobs::detail_scroll_max(app);
            nudge(&mut app.jobs.detail_scroll, delta * 3, max);
        } else if contains(areas.table, x, y) {
            let room = super::jobs::table_room(body, &app.jobs);
            let max = app.jobs.rows.len().saturating_sub(room) as u16;
            nudge(&mut app.jobs.scroll, delta * 3, max);
        }
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
        Region::Detail => {
            let max = if app.module == Some(ModuleId::Providers) {
                provider_catalog_scroll_max(app)
            } else {
                10_000
            };
            nudge(&mut app.scrolls.detail, delta * 3, max);
        }
        Region::Recall => nudge(&mut app.scrolls.recall, delta * 3, 10_000),
        Region::Path => nudge(&mut app.scrolls.path, delta, 10_000),
        Region::Related => {
            let area = detail_areas(app).related;
            let max = super::brain_detail::related_scroll_max(&app.brain_detail.related, area);
            nudge(&mut app.brain_detail.related.scroll, delta * 2, max);
        }
        Region::Summary => nudge(&mut app.scrolls.summary, delta * 3, 10_000),
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
        Region::IntelRelated => {
            let (_, center, _) = intel_briefing_areas(app, body);
            if let Some(layout) = intel_center_layout(app, center) {
                let rows = super::related_articles::rows(
                    &app.intel_related,
                    center.width.saturating_sub(2) as usize,
                    false,
                );
                let max = rows
                    .len()
                    .saturating_sub(layout.related.height.saturating_sub(3) as usize)
                    as u16;
                let before = app.intel_related.scroll;
                nudge(&mut app.intel_related.scroll, delta * 3, max);
                if app.intel_related.scroll == before {
                    let stack_max = intel_brief_scroll_max(app);
                    nudge(&mut app.scrolls.intel_brief, delta * 3, stack_max);
                }
            }
        }
        Region::IntelFull => {
            let max = intel_full_scroll_max(app);
            let before = app.scrolls.intel_full;
            nudge(&mut app.scrolls.intel_full, delta * 3, max);
            if app.scrolls.intel_full == before {
                let stack_max = intel_brief_scroll_max(app);
                nudge(&mut app.scrolls.intel_brief, delta * 3, stack_max);
            }
        }
        Region::IntelExtracted => {
            let max = intel_extracted_scroll_max(app);
            nudge(&mut app.scrolls.intel_extracted, delta * 3, max);
        }
        Region::IntelBrief => {
            let max = intel_brief_scroll_max(app);
            nudge(&mut app.scrolls.intel_brief, delta * 3, max);
        }
        Region::ReconContext => shift_recon_context(app, delta * 3),
        Region::None => {}
    }
}

pub fn page(app: &mut App, direction: i32) {
    if app.overlay == Overlay::None
        && app.module == Some(ModuleId::Brain)
        && app.brain_list_mode == BrainListMode::Topology
    {
        ensure_registered_layout(app);
        let target = app.focus;
        let registry = app.layout.borrow();
        let room = if matches!(
            target,
            Target::TopologyRelated(_) | Target::TopologyRelationships
        ) {
            registry
                .entries
                .iter()
                .filter(|entry| matches!(entry.target, Target::TopologyRelated(_)))
                .count()
                .max(1)
        } else {
            let target = if matches!(
                target,
                Target::TopologyGraph | Target::TopologyNode(_) | Target::TopologyEdge(_)
            ) {
                Target::TopologyGraph
            } else {
                target
            };
            registry
                .entries
                .iter()
                .find(|entry| entry.target == target)
                .map_or(1, |entry| entry.rect.height.max(1) as usize)
        };
        drop(registry);
        super::topology::scroll_topology(app, target, direction as isize * room as isize);
        return;
    }
    if app.overlay == Overlay::AddFallback {
        let rows = add_fallback_layout(active_popup_area(app, app.screen));
        let room =
            super::components::ListGeometry::capacity(inset(rows[3]).height as usize, 1).max(1);
        let last = app.filtered_fallback_models().len().saturating_sub(1);
        app.fallback_popup_sel = app
            .fallback_popup_sel
            .saturating_add_signed(direction as isize * room as isize)
            .min(last);
        reveal_spaced_lists(app);
        return;
    }

    if app.module.is_none() && app.overlay == Overlay::None {
        let body = chrome(app.screen, app).body;
        let room = super::components::ListGeometry::capacity_with_gap(
            body.bottom()
                .saturating_sub(home_layout_metrics(body).apps_y) as usize,
            1,
            0,
        )
        .max(1);
        let index = app
            .launcher_sel
            .saturating_add_signed(direction as isize * room as isize * 2)
            .min(ModuleId::ALL.len() - 1);
        app.set_focus(Target::App(index));
        return;
    }

    if let Overlay::Choice(_) = app.overlay {
        let room = choice_list_room(app).max(1) as i32;
        app.move_choice(direction * room);
        return;
    }
    if app.overlay != Overlay::None {
        let room = inset(active_popup_area(app, app.screen)).height.max(1) as i32;
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
        Some(ModuleId::Recon) if app.focus == Target::ReconContext => {
            shift_recon_context(
                app,
                direction * chrome(app.screen, app).body.height.max(1) as i32,
            );
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
        Some(ModuleId::Brain) if app.brain_list_mode == BrainListMode::Graph => match app.focus {
            Target::RelatedRow(_) => {
                let area = detail_areas(app).related;
                let max = super::brain_detail::related_scroll_max(&app.brain_detail.related, area);
                let room = (super::brain_detail::inner(area).height / 2).max(1) as i32;
                nudge(&mut app.brain_detail.related.scroll, direction * room, max);
            }
            Target::DetailPath => nudge(&mut app.scrolls.path, direction * 4, 10_000),
            _ => nudge(&mut app.scrolls.summary, direction * 4, 10_000),
        },
        Some(ModuleId::Brain) => {}
        Some(ModuleId::Osint) if matches!(app.focus, Target::Tool(_) | Target::ToolCategory(_)) => {
            let room = (tool_room(app) / 2).max(1) as i32;
            let max = tool_max(app);
            nudge_list(&mut app.scrolls.tools, direction * room, max);
        }
        Some(ModuleId::Osint) if app.focus == Target::OsintResponse => {
            app.response_scroll = app
                .response_scroll
                .saturating_add_signed((direction * 6) as isize);
        }
        Some(ModuleId::Osint) if app.focus == Target::OsintDetail => {
            nudge(&mut app.scrolls.detail, direction * 6, 10_000);
        }
        Some(ModuleId::Providers)
            if app.provider_page == ProviderPage::Defaults
                && matches!(
                    app.focus,
                    Target::Button(ButtonId::DefaultRole(_) | ButtonId::FallbackItem(_))
                ) =>
        {
            let body = chrome(app.screen, app).body;
            let models = model_areas(app, provider_areas(body)[1]);
            match app.focus {
                Target::Button(ButtonId::DefaultRole(role)) => {
                    let index = DefaultsRole::ALL
                        .iter()
                        .position(|r| *r == role)
                        .unwrap_or(0);
                    let room = super::components::ListGeometry::capacity(
                        models[6].height.saturating_sub(2) as usize,
                        1,
                    )
                    .max(1);
                    let next = index
                        .saturating_add_signed(direction as isize * room as isize)
                        .min(DefaultsRole::ALL.len() - 1);
                    app.set_focus(Target::Button(ButtonId::DefaultRole(
                        DefaultsRole::ALL[next],
                    )));
                }
                Target::Button(ButtonId::FallbackItem(index)) => {
                    let count = app
                        .settings
                        .defaults
                        .role(app.defaults_role.role_key())
                        .map_or(0, |r| r.fallbacks.len());
                    let room = super::components::ListGeometry::capacity(
                        inset(models[4]).height as usize,
                        1,
                    )
                    .max(1);
                    let next = index
                        .saturating_add_signed(direction as isize * room as isize)
                        .min(count.saturating_sub(1));
                    app.set_focus(Target::Button(ButtonId::FallbackItem(next)));
                }
                _ => {}
            }
        }
        Some(ModuleId::Providers) if app.provider_page != ProviderPage::Defaults => {
            let max = provider_catalog_scroll_max(app);
            nudge(&mut app.scrolls.detail, direction * 6, max);
        }
        Some(ModuleId::Osint) | Some(ModuleId::Providers) => {
            nudge(&mut app.scrolls.detail, direction * 6, 10_000);
        }
        Some(ModuleId::System) => {}
        Some(ModuleId::Logs) => {
            let (room, width) = super::logs::list_geometry(chrome(app.screen, app).body);
            let max = app.logs.scroll_max(room, width);
            nudge(&mut app.logs.scroll, direction * room as i32, max);
        }
        Some(ModuleId::Jobs) if matches!(app.focus, Target::JobDetail) || app.jobs.detail_open => {
            let room = inset(super::jobs::areas(chrome(app.screen, app).body, &app.jobs).detail)
                .height
                .max(1) as i32;
            let max = super::jobs::detail_scroll_max(app);
            nudge(&mut app.jobs.detail_scroll, direction * room, max);
        }
        Some(ModuleId::Jobs) => {
            let body = chrome(app.screen, app).body;
            let room = super::jobs::table_room(body, &app.jobs);
            let max = app.jobs.rows.len().saturating_sub(room) as u16;
            nudge(&mut app.jobs.scroll, direction * room as i32, max);
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
            if matches!(
                app.focus,
                Target::IntelRelatedArticle(_)
                    | Target::Button(ButtonId::IntelRelatedMore | ButtonId::RelatedScope)
            ) {
                let (_, center, _) = intel_briefing_areas(app, chrome(app.screen, app).body);
                if let Some(layout) = intel_center_layout(app, center) {
                    let rows = super::related_articles::rows(
                        &app.intel_related,
                        center.width.saturating_sub(2) as usize,
                        false,
                    );
                    let max = rows
                        .len()
                        .saturating_sub(layout.related.height.saturating_sub(3) as usize)
                        as u16;
                    let before = app.intel_related.scroll;
                    nudge(&mut app.intel_related.scroll, direction * 6, max);
                    if app.intel_related.scroll == before {
                        let stack_max = intel_brief_scroll_max(app);
                        nudge(&mut app.scrolls.intel_brief, direction * 6, stack_max);
                    }
                }
            } else if matches!(
                app.focus,
                Target::IntelLeftColumn
                    | Target::IntelTopologyClaim(_)
                    | Target::IntelTopologyInference(_)
                    | Target::IntelTopologyActor(_)
            ) {
                let max = intel_extracted_scroll_max(app);
                nudge(&mut app.scrolls.intel_extracted, direction * 6, max);
            } else if app.focus == Target::IntelFullArticle {
                let max = intel_full_scroll_max(app);
                let before = app.scrolls.intel_full;
                nudge(&mut app.scrolls.intel_full, direction * 6, max);
                if before == app.scrolls.intel_full {
                    let max = intel_brief_scroll_max(app);
                    nudge(&mut app.scrolls.intel_brief, direction * 6, max);
                }
            } else {
                let max = intel_brief_scroll_max(app);
                nudge(&mut app.scrolls.intel_brief, direction * 6, max);
            }
        }
        Some(ModuleId::Intel) if app.focus == Target::IntelPreview => {
            app.intel_preview_scroll = app
                .intel_preview_scroll
                .saturating_add_signed((direction * 6) as isize);
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
    let next = current.saturating_add_signed(delta as isize).min(max);
    app.scrolls.chat = next;
    app.chat_follow = next == max;
}

pub(crate) fn nudge(scroll: &mut u16, delta: i32, max: u16) {
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
    AtlasOrigins,
    AtlasInsights,
    AtlasFeed,
    AtlasRuns,
    AtlasNews,
    Path,
    Related,
    Summary,
    IntelExtracted,
    IntelBrief,
    IntelFull,
    IntelRelated,
    ReconContext,
    None,
}

fn region_at(app: &App, x: u16, y: u16) -> Region {
    let body = chrome(app.screen, app).body;
    match app.module {
        Some(ModuleId::Recon) if app.recon_chat => {
            let (transcript, context, _) = recon_chat_areas(app, body);
            if contains(transcript, x, y) {
                Region::Chat
            } else if context.is_some_and(|c| contains(c, x, y)) {
                Region::ReconContext
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
            let areas = detail_areas(app);
            if contains(areas.path, x, y) {
                Region::Path
            } else if contains(areas.related, x, y) {
                Region::Related
            } else if contains(areas.summary, x, y) {
                Region::Summary
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
            let layout = osint_areas(app, body, api_key_slot(app).is_some());
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
        Some(ModuleId::System) | Some(ModuleId::Jobs) | Some(ModuleId::Logs) => Region::None,
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
                let (_, table, insights, feed) = atlas_live_areas(app, body);
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
            let (left, center, _right) = intel_briefing_areas(app, body);
            if contains(left, x, y) {
                Region::IntelExtracted
            } else if !contains(center, x, y) {
                Region::None
            } else if let Some(layout) = intel_center_layout(app, center) {
                if abs_contains(layout.related, x, y) {
                    Region::IntelRelated
                } else if abs_contains(layout.full, x, y) {
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
    super::components::ListGeometry::capacity(
        brain_list(chrome(app.screen, app).body)
            .list
            .height
            .saturating_sub(2) as usize,
        2,
    )
    .max(1)
}

fn tool_room(app: &App) -> usize {
    list_room(
        osint_areas(
            app,
            chrome(app.screen, app).body,
            api_key_slot(app).is_some(),
        )
        .list
        .height,
    )
    .max(1)
}

fn thread_max(app: &App) -> u16 {
    app.threads.len().saturating_sub(thread_room(app).max(1)) as u16
}

fn tool_max(app: &App) -> u16 {
    super::tool_catalog::rows(&app.osint_search, &app.tool_expanded)
        .len()
        .saturating_sub(tool_room(app).max(1)) as u16
}

pub(super) fn detail_rows(detail: &str, width: usize) -> Vec<String> {
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

fn popup_max(app: &App) -> u16 {
    let inner = inset(active_popup_area(app, app.screen));
    popup_lines(app, inner.width as usize)
        .len()
        .saturating_sub(inner.height.max(1) as usize)
        .min(u16::MAX as usize) as u16
}

fn atlas_feed_room(app: &App) -> usize {
    super::components::ListGeometry::capacity_with_gap(
        inset(atlas_live_areas(app, chrome(app.screen, app).body).3)
            .height
            .max(1) as usize,
        1,
        0,
    )
    .max(1)
}

fn origins_room(app: &App) -> usize {
    inset(atlas_live_areas(app, chrome(app.screen, app).body).1)
        .height
        .saturating_sub(1)
        .max(1) as usize
}

fn insights_room(app: &App) -> usize {
    inset(atlas_live_areas(app, chrome(app.screen, app).body).2)
        .height
        .saturating_sub(2)
        .max(1) as usize
}

fn origins_scroll_max(app: &App) -> u16 {
    let inner = inset(atlas_live_areas(app, chrome(app.screen, app).body).1);
    let origins: Vec<_> = app
        .atlas_stats
        .origins
        .iter()
        .filter(|row| row.articles > 0 || !app.atlas_stats.scored)
        .collect();
    super::atlas_table::origins_lines(&origins, &app.atlas_stats, inner.width)
        .len()
        .saturating_sub(inner.height as usize)
        .min(u16::MAX as usize) as u16
}

fn atlas_cycle_stats_scroll_max(app: &App) -> u16 {
    let stats = selected_cycle_stats(app);
    let inner = inset(atlas_runs_areas(chrome(app.screen, app).body).2);
    let origins: Vec<_> = stats
        .origins
        .iter()
        .filter(|row| row.articles > 0)
        .collect();
    super::atlas_table::origins_lines(&origins, &stats, inner.width)
        .len()
        .saturating_sub(atlas_cycle_stats_room(app))
        .min(u16::MAX as usize) as u16
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
    super::components::ListGeometry::capacity_with_gap(
        inset(atlas_runs_areas(chrome(app.screen, app).body).3)
            .height
            .max(1) as usize,
        1,
        0,
    )
    .max(1)
}

fn atlas_cycle_stats_room(app: &App) -> usize {
    inset(atlas_runs_areas(chrome(app.screen, app).body).2)
        .height
        .saturating_sub(3)
        .max(1) as usize
}

fn atlas_news_room(app: &App) -> usize {
    super::components::ListGeometry::capacity_with_gap(
        inset(atlas_news_areas(chrome(app.screen, app).body).2)
            .height
            .max(1) as usize,
        1,
        0,
    )
    .max(1)
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

#[allow(dead_code)]
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

/// Test-only reader geometry for the Intel brief focus view.
#[cfg(test)]
#[derive(Clone, Copy, Debug)]
pub struct IntelCenterGeometry {
    pub image: (u16, i32, u16, u16),
    pub brief: (u16, i32, u16, u16),
    pub brief_text: (u16, i32, u16, u16),
}

#[cfg(test)]
#[allow(dead_code)]
pub fn intel_center_geometry_for(app: &App) -> Option<IntelCenterGeometry> {
    let body = chrome(app.screen, app).body;
    let (_left, center, _right) = intel_briefing_areas(app, body);
    let layout = intel_center_layout(app, center)?;
    let rect = |area: AbsRect| (area.x, area.y, area.width, area.height);
    Some(IntelCenterGeometry {
        image: rect(layout.image),
        brief: rect(layout.brief),
        brief_text: rect(layout.brief_text),
    })
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

fn topology_geometry_stamp(state: &super::topology::TopologyState) -> String {
    use argos_osint_core::brain_topology::CycleScope;
    use std::hash::{Hash, Hasher};

    // A snapshot's vector order owns its numeric targets. Include the complete
    // geometry inputs, but omit full source passages and detail prose: their
    // viewport is fixed and a publication's sequence identifies its revision.
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    let cycle = match &state.cycle_scope {
        CycleScope::AllRetained => None,
        CycleScope::ThisCycle(id) => Some(id.as_str()),
    };
    (
        state.seed.as_ref().map(|seed| seed.key()),
        state.depth,
        cycle,
        state.filters(),
        state.zoom_percent,
        state.pan,
        &state.viewport_center_id,
    )
        .hash(&mut hash);
    (
        state.active_panel_tab,
        state.inspector_tab,
        state.inspector_scroll,
        state.related_sel,
        state.related_scroll,
        &state.selected_node_id,
        &state.selected_edge_id,
        state.analyzing_scope,
        &state.scope_status,
    )
        .hash(&mut hash);
    state.snapshot.is_some().hash(&mut hash);
    if let Some(snapshot) = &state.snapshot {
        (
            &snapshot.root_id,
            &snapshot.root_label,
            snapshot.root_kind.map(|kind| kind.label()),
            snapshot.memory_changed_seq,
            snapshot.nodes.len(),
            snapshot.edges.len(),
        )
            .hash(&mut hash);
        for node in &snapshot.nodes {
            (
                &node.id,
                node.kind.label(),
                node.min_hops,
                &node.label,
                &node.status,
                node.is_root,
                &node.revision,
                &node.member_ids,
            )
                .hash(&mut hash);
        }
        for edge in &snapshot.edges {
            (
                &edge.id,
                &edge.from,
                &edge.to,
                edge.kind.label(),
                &edge.label,
                &edge.status,
            )
                .hash(&mut hash);
        }
    }
    format!("{:016x}", hash.finish())
}

fn geometry_stamp(app: &App) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}",
        (
            app.module,
            &app.overlay,
            app.screen,
            app.focus,
            app.brain_list_mode,
            app.provider_page,
            app.intel_page,
            app.atlas_page
        ),
        (
            app.recon_chat,
            app.tool_sel,
            &app.osint_search,
            app.scrolls.chat,
            app.scrolls.threads,
            app.scrolls.memories,
            app.scrolls.tools,
            app.scrolls.intel_list,
            app.scrolls.atlas_feed,
            (
                app.scrolls.atlas_runs,
                app.scrolls.atlas_news,
                app.scrolls.detail,
                app.scrolls.recon_context,
                app.jobs.detail_scroll,
                app.intel_related.scroll,
                app.scrolls.intel_brief,
                app.scrolls.intel_full,
                app.scrolls.intel_extracted,
            )
        ),
        (
            app.profile.focus,
            app.profile.report,
            app.profile.filter_popup,
            app.profile.period_popup,
            app.profile.tab,
            app.profile.all_apps,
            app.compact_pages,
            app.profile.section,
            app.profile.grid_scroll.offset
        ),
        (
            (
                (
                    app.logs.scroll,
                    &app.logs.open,
                    app.logs.rows.len(),
                    app.logs.error.is_some()
                ),
                app.scrolls.home,
                app.scrolls.roles,
                app.scrolls.fallbacks,
                app.scrolls.popup
            ),
            (
                app.jobs.scroll,
                app.jobs.detail_open,
                app.jobs.rows.len(),
                app.jobs.error.is_some()
            ),
            (
                app.profile
                    .snapshot
                    .as_ref()
                    .map_or("", |snapshot| snapshot.captured_at.as_str()),
                app.profile.report_scroll.offset,
                &app.profile.table_offsets,
                &app.profile.panel_scroll,
                &app.profile.selected_row,
                app.intel_sel,
            ),
            app.recon_context_enabled,
            app.profile.picker_selection,
            &app.profile.filter_edit
        ),
        (app.module == Some(ModuleId::Brain) && app.brain_list_mode == BrainListMode::Topology)
            .then(|| (
                topology_geometry_stamp(&app.topology_state),
                app.topology_loading,
                &app.topology_error,
                &app.topology_origin_label,
            )),
    )
}
fn ensure_registered_layout(app: &App) {
    let stamp = geometry_stamp(app);
    if app.layout.borrow().view_stamp == stamp || app.screen.width == 0 || app.screen.height == 0 {
        return;
    }
    // Render into a cell buffer to update the exact component registrations
    // before an input event that follows a state transition without a paint.
    // No per-cell hit scan and no storage reads are involved.
    let backend = ratatui::backend::TestBackend::new(app.screen.width, app.screen.height);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    let _ = terminal.draw(|frame| draw(frame, app));
}

pub fn focus_order(app: &App) -> Vec<Target> {
    ensure_registered_layout(app);
    let registry = app.layout.borrow();
    let mut entries = registry.entries.clone();
    entries.retain(|e| {
        e.scope == registry.current_scope
            && !matches!(
                e.target,
                Target::ProfileBucket(_)
                    | Target::ProfilePanelBucket(_, _)
                    | Target::ProfilePanelRow(_, _)
                    | Target::ProfileRow(_)
                    | Target::ProfileTableRow(_, _)
            )
    });
    drop(registry);
    if app.overlay != Overlay::None && entries.is_empty() {
        return vec![Target::CloseOverlay];
    }
    entries.sort_by_key(|e| (e.rect.y, e.rect.x));
    if app.module == Some(ModuleId::System)
        && app.overlay == Overlay::None
        && app.profile.tab == super::profile::SystemTab::Overview
        && app.profile.report.is_none()
        && app.profile.filter_popup.is_none()
        && !app.profile.period_popup
    {
        let mut order: Vec<_> = entries
            .iter()
            .map(|e| e.target)
            .filter(|t| {
                !matches!(
                    t,
                    Target::ProfileCard(_)
                        | Target::ProfilePanelBucket(_, _)
                        | Target::ProfilePanelRow(_, _)
                        | Target::ProfileBucket(_)
                        | Target::ProfileRow(_)
                )
            })
            .collect();
        order.extend((0..app.profile.focused_widgets().len()).map(Target::ProfileCard));
        return order;
    }

    if app.module == Some(ModuleId::System)
        && app.overlay == Overlay::None
        && app.profile.report.is_some()
    {
        let mut order = vec![Target::ProfileReport];
        order.extend((0..super::profile::report_table_count(app)).map(Target::ProfileTable));
        order.extend(
            entries
                .iter()
                .filter(|entry| matches!(entry.target, Target::ProfileAction(_)))
                .map(|entry| entry.target),
        );
        return order;
    }
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        if seen.insert(entry.target) {
            order.push(entry.target);
        }
    }
    if app.module.is_none() && app.overlay == Overlay::None {
        order.retain(|target| !matches!(target, Target::App(_)));
        order.extend((0..ModuleId::ALL.len()).map(Target::App));
    }
    if app.module == Some(ModuleId::Providers)
        && app.provider_page == ProviderPage::Defaults
        && app.overlay == Overlay::None
    {
        for role in DefaultsRole::ALL {
            let target = Target::Button(ButtonId::DefaultRole(role));
            if !order.contains(&target) {
                order.push(target);
            }
        }
        let count = app
            .settings
            .defaults
            .role(app.defaults_role.role_key())
            .map_or(0, |role| role.fallbacks.len());
        for index in 0..count {
            let target = Target::Button(ButtonId::FallbackItem(index));
            if !order.contains(&target) {
                order.push(target);
            }
        }
    }
    // Intel's middle stack can initially clip Related completely. Its logical
    // scope control enters focus order; set_focus reveals the same layout pane.
    if app.module == Some(ModuleId::Intel)
        && app.intel_page == IntelPage::Briefing
        && app.overlay == Overlay::None
    {
        let (left, center, _) = intel_briefing_areas(app, chrome(app.screen, app).body);
        if let Some(layout) = intel_extracted_layout(app, left) {
            order.retain(|target| {
                !matches!(
                    target,
                    Target::IntelTopologyClaim(_)
                        | Target::IntelTopologyInference(_)
                        | Target::IntelTopologyActor(_)
                )
            });
            let at = order
                .iter()
                .position(|target| *target == Target::IntelLeftColumn)
                .map_or(order.len(), |index| index + 1);
            order.splice(
                at..at,
                layout
                    .actor_records
                    .iter()
                    .chain(&layout.claim_records)
                    .chain(&layout.inference_records)
                    .map(|(_, target)| *target),
            );
        }
        let target = Target::Button(ButtonId::RelatedScope);
        if center.width > 0 && !seen.contains(&target) {
            order.push(target);
        }
    }
    if app.overlay != Overlay::None && !seen.contains(&Target::CloseOverlay) {
        order.push(Target::CloseOverlay);
    }
    order
}

pub fn choice_list_room(app: &App) -> usize {
    let inner = inset(popup_area(app.screen));
    let note = usize::from(!app.choice_note.is_empty());
    super::components::ListGeometry::capacity(inner.height.saturating_sub(note as u16) as usize, 1)
}

pub fn hit_test(app: &App, x: u16, y: u16) -> Option<Target> {
    ensure_registered_layout(app);
    let registry = app.layout.borrow();
    for entry in registry.entries.iter().rev() {
        if entry.scope == registry.current_scope && contains(entry.rect, x, y) {
            return Some(entry.target);
        }
    }
    drop(registry);
    if app.overlay == Overlay::None {
        legacy_target_at(app, x, y)
    } else {
        None
    }
}

fn legacy_target_at(app: &App, x: u16, y: u16) -> Option<Target> {
    let layout = chrome(app.screen, app);
    if app.module.is_none() {
        let composer = home_composer_areas(layout.body);
        if contains(composer.send, x, y) {
            return Some(Target::Button(ButtonId::Send));
        }
        if contains(composer.input, x, y) {
            return Some(Target::Field(FieldId::Composer));
        }
    } else if layout.composer.height > 0 {
        let composer = recon_composer_areas(layout.composer);
        if contains(composer.send, x, y) {
            return Some(Target::Button(ButtonId::Send));
        }
        if contains(composer.input, x, y) {
            return Some(Target::Field(FieldId::Composer));
        }
    }
    let tabs = if app.module.is_none() {
        layout.header
    } else {
        layout.tab_strip
    };
    if contains(tabs, x, y) {
        if let Some(target) = tab_strip_hit(app, tabs, x, y) {
            return Some(target);
        }
    }
    if !contains(layout.body, x, y) {
        return None;
    }
    match app.module {
        None => None,
        Some(ModuleId::Recon) => recon_hit(app, layout.body, x, y),
        Some(ModuleId::Brain) => brain_hit(app, layout.body, x, y),
        Some(ModuleId::Osint) => osint_hit(app, layout.body, x, y),
        Some(ModuleId::Atlas) => atlas_hit(app, layout.body, x, y),
        Some(ModuleId::Intel) => intel_hit(app, layout.body, x, y),
        Some(ModuleId::Providers) => provider_hit(app, layout.body, x, y),
        Some(ModuleId::System) => system_hit(app, layout.body, x, y),
        Some(ModuleId::Jobs) => {
            super::jobs::hit(app, layout.body, x, y, app.job_source().is_some())
        }
        Some(ModuleId::Logs) => super::logs::hit(app, layout.body, x, y),
    }
}

fn recon_hit(app: &App, body: Rect, x: u16, y: u16) -> Option<Target> {
    if app.recon_chat {
        let (transcript, context, bottom_actions) = recon_chat_areas(app, body);
        if let Some(context_area) = context {
            for (btn_id, rect, _) in recon_context_buttons(context_area, app) {
                if contains(rect, x, y) {
                    return Some(Target::Button(btn_id));
                }
            }
        } else if let Some(run_actions) = bottom_actions {
            let mut buttons = vec![ButtonId::RetryInsights];
            if app.can_resume_recon() {
                buttons.push(ButtonId::ResumeRun);
            }
            if app.can_stop_recon() {
                buttons.push(ButtonId::CancelRun);
            }
            let areas = button_areas(run_actions, buttons.len());
            for (i, id) in buttons.into_iter().enumerate() {
                if contains(areas[i], x, y) {
                    return Some(Target::Button(id));
                }
            }
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
        let index = app.scrolls.threads as usize
            + super::components::ListGeometry::fixed(app.threads.len(), 1)
                .index_at((y - list.y - 1) as usize)?;
        if index < app.threads.len() {
            return Some(Target::Thread(index));
        }
    }
    if contains(actions, x, y) {
        let areas = button_areas(actions, 1);
        if contains(areas[0], x, y) {
            return Some(Target::Button(ButtonId::DeleteThread));
        }
    }
    None
}

fn brain_hit(app: &App, body: Rect, x: u16, y: u16) -> Option<Target> {
    if app.brain_list_mode == BrainListMode::Topology {
        return app.layout.borrow().target_at(x, y);
    }
    if app.brain_list_mode == BrainListMode::Graph {
        let areas = super::brain_detail::areas_for(app, body);
        if contains(areas.back, x, y) {
            return Some(Target::Button(ButtonId::BrainDetailBack));
        }
        if contains(areas.path, x, y) {
            return Some(Target::DetailPath);
        }
        if contains(areas.related, x, y) {
            return super::brain_detail::related_at(&app.brain_detail.related, areas.related, x, y)
                .map(Target::RelatedRow);
        }
        if contains(areas.summary, x, y) {
            if let Some(failure) = &app.summary_failure {
                let inner = super::graph::summary_inner(areas.summary);
                if let Some(button) = super::summary_card::button_at(app, failure, inner, x, y) {
                    return Some(Target::Button(button));
                }
            }
            return Some(Target::DetailSummary);
        }
        return None;
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
        let buttons = button_areas(layout.actions, 4);
        let ids = [
            ButtonId::CreateMemory,
            ButtonId::Pin,
            ButtonId::Delete,
            ButtonId::BrainKind,
        ];
        return buttons
            .iter()
            .position(|rect| contains(*rect, x, y))
            .map(|index| Target::Button(ids[index]));
    }
    if in_pane(layout.list, x, y) {
        let index = app.scrolls.memories as usize
            + super::components::ListGeometry::fixed(app.memories.len(), 2)
                .index_at((y - layout.list.y - 1) as usize)?;
        if index < app.memories.len() {
            return Some(Target::Memory(index));
        }
    }
    None
}

fn osint_buttons(
    tool_id: Option<&str>,
    refresh_running: Option<&str>,
) -> Vec<(ButtonId, &'static str)> {
    match tool_id {
        Some("whatsmyname_lookup") => {
            let refresh_lbl = if refresh_running == Some("whatsmyname") {
                "Refreshing…"
            } else {
                "Refresh"
            };
            vec![
                (ButtonId::OsintRun, "Run"),
                (ButtonId::OsintCancel, "Cancel"),
                (ButtonId::OsintRefreshDataset, refresh_lbl),
                (ButtonId::OsintToggle, "Enable"),
                (ButtonId::OsintRaw, "Raw"),
                (ButtonId::OsintAttach, "Attach"),
                (ButtonId::OsintStartRecon, "Recon"),
                (ButtonId::OsintPrev, "Prev"),
                (ButtonId::OsintNext, "Next"),
                (ButtonId::OpenDocumentation, "Docs"),
            ]
        }
        Some("dork_generate") => {
            let refresh_lbl = if refresh_running == Some("dorksearch") {
                "Refreshing…"
            } else {
                "Refresh"
            };
            vec![
                (ButtonId::OsintRun, "Generate"),
                (ButtonId::OsintSearchSelected, "Search"),
                (ButtonId::OsintRefreshTemplates, refresh_lbl),
                (ButtonId::OsintCancel, "Cancel"),
                (ButtonId::OsintToggle, "Enable"),
                (ButtonId::OsintRaw, "Raw"),
                (ButtonId::OsintAttach, "Attach"),
                (ButtonId::OsintStartRecon, "Recon"),
                (ButtonId::OsintPrev, "Prev"),
                (ButtonId::OsintNext, "Next"),
            ]
        }
        Some("whoxy_whois_history") => vec![
            (ButtonId::OsintRun, "Run"),
            (ButtonId::TestWhoxyConnection, "Test"),
            (ButtonId::OsintCancel, "Cancel"),
            (ButtonId::OsintToggle, "Enable"),
            (ButtonId::OsintRaw, "Raw"),
            (ButtonId::OsintAttach, "Attach"),
            (ButtonId::OsintStartRecon, "Recon"),
            (ButtonId::OsintPrev, "Prev"),
            (ButtonId::OsintNext, "Next"),
            (ButtonId::OpenDocumentation, "Docs"),
        ],
        _ => vec![
            (ButtonId::OsintRun, "Run"),
            (ButtonId::OsintCancel, "Cancel"),
            (ButtonId::OsintToggle, "Enable"),
            (ButtonId::OsintRaw, "Raw"),
            (ButtonId::OsintAttach, "Attach"),
            (ButtonId::OsintStartRecon, "Recon"),
            (ButtonId::OsintPrev, "Prev"),
            (ButtonId::OsintNext, "Next"),
            (ButtonId::OpenDocumentation, "Docs"),
        ],
    }
}

fn osint_hit(app: &App, body: Rect, x: u16, y: u16) -> Option<Target> {
    let slot = api_key_slot(app);
    let layout = osint_areas(app, body, slot.is_some());
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
        let rows = super::tool_catalog::rows(&app.osint_search, &app.tool_expanded);
        let index = app.scrolls.tools as usize
            + super::components::ListGeometry::fixed(rows.len(), 1)
                .index_at((y - list.y - 1) as usize)?;
        return rows.get(index).map(|row| row.0);
    }
    if contains(input, x, y) {
        return Some(Target::Field(FieldId::OsintInput));
    }
    if contains(actions, x, y) {
        let tool = osint::registry().get(app.tool_sel);
        let buttons = osint_buttons(tool.map(|t| t.id), app.dataset_refresh_running.as_deref());
        let areas = button_areas(actions, buttons.len());
        return areas
            .iter()
            .position(|rect| contains(*rect, x, y))
            .map(|index| Target::Button(buttons[index].0));
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
        ProviderPage::Google | ProviderPage::Nvidia | ProviderPage::OpenRouter => {
            let (key_field, ep_field, btn_save, btn_verify, btn_advanced, advanced) =
                match app.provider_page {
                    ProviderPage::Google => (
                        FieldId::GoogleKey,
                        FieldId::GoogleEndpoint,
                        ButtonId::GoogleSave,
                        ButtonId::GoogleVerify,
                        ButtonId::GoogleAdvanced,
                        app.google_advanced,
                    ),
                    ProviderPage::Nvidia => (
                        FieldId::NvidiaKey,
                        FieldId::NvidiaEndpoint,
                        ButtonId::NvidiaSave,
                        ButtonId::NvidiaVerify,
                        ButtonId::NvidiaAdvanced,
                        app.nvidia_advanced,
                    ),
                    ProviderPage::OpenRouter => (
                        FieldId::RouterKey,
                        FieldId::RouterEndpoint,
                        ButtonId::RouterSave,
                        ButtonId::RouterVerify,
                        ButtonId::RouterAdvanced,
                        app.router_advanced,
                    ),
                    _ => unreachable!(),
                };
            let router = router_areas(rows[1]);
            if contains(router[1], x, y) {
                return Some(Target::Field(key_field));
            }
            if contains(router[2], x, y) {
                let buttons = button_areas(router[2], 2);
                if contains(buttons[0], x, y) {
                    return Some(Target::Button(btn_save));
                }
                if contains(buttons[1], x, y) {
                    return Some(Target::Button(btn_verify));
                }
            }
            if contains(router[3], x, y) {
                return Some(Target::Button(btn_advanced));
            }
            if advanced && contains(router[4], x, y) {
                return Some(Target::Field(ep_field));
            }
            let catalog_area = split_vertical(
                router[5],
                [
                    Constraint::Length(FIELD_H),
                    Constraint::Length(ACTION_H),
                    Constraint::Min(0),
                ],
            );
            let filter_field = match app.provider_page {
                ProviderPage::Google => FieldId::GoogleModelFilter,
                ProviderPage::Nvidia => FieldId::NvidiaModelFilter,
                ProviderPage::OpenRouter => FieldId::RouterModelFilter,
                ProviderPage::Defaults => unreachable!(),
            };
            if contains(catalog_area[0], x, y) {
                return Some(Target::Field(filter_field));
            }
            if contains(catalog_area[1], x, y) {
                return Some(Target::Button(ButtonId::RefreshModels));
            }
        }
        ProviderPage::Defaults => {
            let models = model_areas(app, rows[1]);
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
                let save = button_areas(models[3], 4);
                for (area, id) in save.into_iter().zip([
                    role.save_button(),
                    ButtonId::BriefOpCycle,
                    ButtonId::BriefOpSave,
                    ButtonId::BriefOpReset,
                ]) {
                    if contains(area, x, y) {
                        return Some(Target::Button(id));
                    }
                }
            }
            if contains(models[4], x, y) {
                let fallbacks = app
                    .settings
                    .defaults
                    .role(role.role_key())
                    .map(|a| a.fallbacks.len())
                    .unwrap_or(0);
                if fallbacks == 0 {
                    return Some(Target::Button(ButtonId::AddFallback));
                }
                let inner = inset(models[4]);
                if inner.height > 0 {
                    let row_h = 1.max(inner.height / fallbacks.max(1) as u16);
                    for i in 0..fallbacks {
                        let y0 = inner.y.saturating_add(i as u16 * row_h);
                        if y >= y0 && y < y0.saturating_add(row_h).min(inner.y + inner.height) {
                            return Some(Target::Button(ButtonId::FallbackItem(i)));
                        }
                    }
                }
            }
            if contains(models[5], x, y) {
                let buttons = button_areas(models[5], 5);
                if contains(buttons[0], x, y) {
                    return Some(Target::Button(ButtonId::AddFallback));
                }
                if contains(buttons[4], x, y) {
                    return Some(Target::Button(ButtonId::BriefFastPreset));
                }
                if contains(buttons[1], x, y) {
                    return Some(Target::Button(ButtonId::DeleteFallback));
                }
                if contains(buttons[2], x, y) {
                    return Some(Target::Button(ButtonId::MoveFallbackUp));
                }
                if contains(buttons[3], x, y) {
                    return Some(Target::Button(ButtonId::MoveFallbackDown));
                }
            }
        }
    }
    None
}

fn system_hit(app: &App, body: Rect, x: u16, y: u16) -> Option<Target> {
    if app.profile.tab != super::profile::SystemTab::System {
        return None;
    }
    let body = Rect::new(
        body.x,
        body.y + 1,
        body.width,
        body.height.saturating_sub(2),
    );
    let (_, actions) = system_areas(body);
    contains(system_button(actions), x, y).then_some(Target::Button(ButtonId::RefreshHardware))
}

pub(crate) fn system_button(actions: Rect) -> Rect {
    let width = 22.min(actions.width);
    Rect { width, ..actions }
}

fn field_rect(app: &App, field: FieldId) -> Option<Rect> {
    let layout = chrome(app.screen, app);
    match field {
        FieldId::Composer if app.module.is_none() => Some(home_composer_areas(layout.body).input),
        FieldId::Composer if layout.composer.height > 0 => Some(composer_parts(layout.composer).0),
        FieldId::ReconSearch if app.module == Some(ModuleId::Recon) && !app.recon_chat => {
            Some(dashboard_areas(layout.body).0)
        }
        FieldId::LogsSearch if app.module == Some(ModuleId::Logs) => {
            Some(super::logs::areas(layout.body).search)
        }
        FieldId::JobsSearch if app.module == Some(ModuleId::Jobs) => {
            Some(super::jobs::areas(layout.body, &app.jobs).search)
        }
        FieldId::OsintSearch if app.module == Some(ModuleId::Osint) => {
            Some(osint_areas(app, layout.body, api_key_slot(app).is_some()).search)
        }
        FieldId::OsintInput if app.module == Some(ModuleId::Osint) => {
            Some(osint_areas(app, layout.body, api_key_slot(app).is_some()).input)
        }
        FieldId::FirecrawlKey
        | FieldId::HunterKey
        | FieldId::SociaVaultKey
        | FieldId::NewsApiKey
        | FieldId::CourtListenerKey
        | FieldId::GnewsKey
        | FieldId::NewsDataKey
        | FieldId::CurrentsKey
        | FieldId::WhoxyKey
            if app.module == Some(ModuleId::Osint)
                && api_key_slot(app).is_some_and(|slot| slot.field == field) =>
        {
            let key = osint_areas(app, layout.body, true).key;
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
        | FieldId::WhoxyFallback
            if app.module == Some(ModuleId::Osint)
                && api_key_slot(app).is_some_and(|slot| slot.fallback == field) =>
        {
            Some(osint_areas(app, layout.body, true).fallback)
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
        field
            if super::app::DefaultsRole::of_field(field).is_some()
                && app.module == Some(ModuleId::Providers)
                && app.provider_page == ProviderPage::Defaults =>
        {
            let rows = model_areas(app, provider_areas(layout.body)[1]);
            Some(
                if super::app::DefaultsRole::ALL
                    .iter()
                    .any(|r| r.provider_field() == field)
                {
                    rows[1]
                } else {
                    rows[2]
                },
            )
        }
        FieldId::RouterKey
        | FieldId::RouterEndpoint
        | FieldId::GoogleKey
        | FieldId::GoogleEndpoint
        | FieldId::NvidiaKey
        | FieldId::NvidiaEndpoint
            if app.module == Some(ModuleId::Providers) =>
        {
            let rows = router_areas(provider_areas(layout.body)[1]);
            Some(
                if matches!(
                    field,
                    FieldId::RouterKey | FieldId::GoogleKey | FieldId::NvidiaKey
                ) {
                    rows[1]
                } else {
                    rows[4]
                },
            )
        }
        FieldId::FallbackFilter if app.overlay == Overlay::AddFallback => {
            Some(add_fallback_layout(add_fallback_popup_area(app.screen))[2])
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
    let prefix: Vec<char> = value.chars().take(cursor).collect();
    let mut start = prefix.len();
    let mut used = 0;
    while start > 0 {
        let ch = prefix[start - 1];
        if ch == '\n' {
            break;
        }
        let cells = super::components::text_width(&ch.to_string());
        if used + cells > width.saturating_sub(1) {
            break;
        }
        used += cells;
        start -= 1;
    }
    start
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
        let start = viewport(app, field, value_area);
        let mut cells = 0;
        let mut cursor = start;
        for ch in value.chars().skip(start) {
            let width = super::components::text_width(&ch.to_string());
            if cells + width > offset {
                break;
            }
            cells += width;
            cursor += 1;
        }
        return cursor;
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

fn count_items_label(count: usize) -> String {
    if count == 1 {
        "1 item".to_string()
    } else {
        format!("{count} items")
    }
}

pub(crate) fn field_placeholder(app: &App, field: FieldId) -> String {
    match field {
        FieldId::BrainApp => "Source app name".into(),
        FieldId::BrainConversation => "Optional conversation ID".into(),
        FieldId::BrainInsight => "Write a useful fact or finding…".into(),
        FieldId::BrainQuery => {
            format!(
                "Search memories by topic ({})…",
                count_items_label(app.memories.len())
            )
        }
        FieldId::ReconSearch => {
            format!(
                "Find investigations ({})…",
                count_items_label(app.threads.len())
            )
        }
        FieldId::IntelSearch => {
            format!(
                "Search title, source, or topic ({})…",
                count_items_label(app.intel_articles.len())
            )
        }
        FieldId::OsintSearch => {
            format!(
                "Find tools by name or purpose ({})…",
                count_items_label(osint::registry().len())
            )
        }
        FieldId::OsintInput => "JSON inputs; see example above".into(),
        FieldId::JobsSearch => {
            format!(
                "Find jobs by name or state ({})…",
                count_items_label(app.jobs.rows.len())
            )
        }
        FieldId::LogsSearch => {
            format!(
                "Filter messages or IDs ({})…",
                count_items_label(app.logs.rows.len())
            )
        }
        FieldId::RouterKey => "Paste OpenRouter API key".into(),
        FieldId::GoogleKey => "Paste Google AI Studio key".into(),
        FieldId::NvidiaKey => "Paste NVIDIA API key".into(),
        FieldId::RouterEndpoint | FieldId::GoogleEndpoint | FieldId::NvidiaEndpoint => {
            "API base URL, including version".into()
        }
        FieldId::GoogleModelFilter => {
            let count = app
                .catalog_cache
                .get("google")
                .map(|c| c.len())
                .unwrap_or_else(|| {
                    if app.catalog_for == "google" {
                        app.model_catalog.len()
                    } else {
                        0
                    }
                });
            format!("Filter model names or IDs ({})…", count_items_label(count))
        }
        FieldId::NvidiaModelFilter => {
            let count = app
                .catalog_cache
                .get("nvidia")
                .map(|c| c.len())
                .unwrap_or_else(|| {
                    if app.catalog_for == "nvidia" {
                        app.model_catalog.len()
                    } else {
                        0
                    }
                });
            format!("Filter model names or IDs ({})…", count_items_label(count))
        }
        FieldId::RouterModelFilter => {
            let count = app
                .catalog_cache
                .get("openrouter")
                .map(|c| c.len())
                .unwrap_or_else(|| {
                    if app.catalog_for == "openrouter" {
                        app.model_catalog.len()
                    } else {
                        0
                    }
                });
            format!("Filter model names or IDs ({})…", count_items_label(count))
        }
        FieldId::FallbackFilter => {
            format!(
                "Filter model names or IDs ({})…",
                count_items_label(app.model_catalog.len())
            )
        }
        FieldId::Composer => "Ask an OSINT question…".into(),
        field if is_picker_field(field) => {
            if matches!(
                field,
                FieldId::ReconProvider
                    | FieldId::PickerProvider
                    | FieldId::SynthesisProvider
                    | FieldId::ClassifierProvider
                    | FieldId::SummarizationProvider
                    | FieldId::EvidenceCuratorProvider
                    | FieldId::EntityResolverProvider
                    | FieldId::ClaimAssessorProvider
                    | FieldId::InvestigationControllerProvider
            ) {
                "Choose a provider".into()
            } else {
                "Choose a model".into()
            }
        }
        FieldId::WhoxyKey => "Whoxy API key".into(),
        FieldId::FirecrawlFallback
        | FieldId::HunterFallback
        | FieldId::SociaVaultFallback
        | FieldId::NewsApiFallback
        | FieldId::CourtListenerFallback
        | FieldId::GnewsFallback
        | FieldId::NewsDataFallback
        | FieldId::CurrentsFallback
        | FieldId::WhoxyFallback => "Optional backup API key".into(),
        _ => "Paste API key".into(),
    }
}

pub(super) fn draw_field(frame: &mut Frame, app: &App, field: FieldId, label: &str, area: Rect) {
    app.layout.borrow_mut().register(Target::Field(field), area);
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
            | FieldId::GoogleKey
            | FieldId::NvidiaKey
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
            | FieldId::WhoxyKey
            | FieldId::WhoxyFallback
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
        field_placeholder(app, field)
    } else {
        String::new()
    };
    let showing_placeholder = !empty.is_empty();
    let center_empty =
        matches!(field, FieldId::BrainQuery | FieldId::IntelSearch) && showing_placeholder;
    let visible: String = if !showing_placeholder {
        super::components::clip_text(&display.chars().skip(scroll).collect::<String>(), width)
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
        let visible_prefix: String = display
            .chars()
            .skip(scroll)
            .take(app.cursor.saturating_sub(scroll))
            .collect();
        let column = if field == FieldId::Composer {
            col
        } else {
            super::components::text_width(&visible_prefix)
        };
        let x = value_area.x + 1 + (column as u16).min(value_area.width.saturating_sub(2));
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

pub(crate) fn draw_button(frame: &mut Frame, app: &App, button: ButtonId, label: &str, area: Rect) {
    draw_button_state(frame, app, button, label, area, false);
}

pub(super) fn draw_button_state(
    frame: &mut Frame,
    app: &App,
    button: ButtonId,
    label: &str,
    area: Rect,
    active: bool,
) {
    if button == ButtonId::IntelVisit
        && !app
            .intel_articles
            .get(app.intel_sel)
            .is_some_and(|article| {
                url::Url::parse(&article.url).is_ok_and(|url| {
                    matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
                })
            })
    {
        frame.render_widget(
            Paragraph::new("Visit disabled · missing/invalid article URL").style(theme::muted()),
            area,
        );
        return;
    }
    app.layout
        .borrow_mut()
        .register(Target::Button(button), area);
    if area.width == 0 || area.height == 0 {
        return;
    }
    if matches!(
        button,
        ButtonId::FallbackTab(_)
            | ButtonId::AtlasWorld
            | ButtonId::AtlasNews
            | ButtonId::BrainRelatedFirst
            | ButtonId::BrainRelatedSecond
            | ButtonId::BrainRelatedThird
    ) {
        super::components::tab_button(
            frame,
            area,
            label,
            active,
            app.focus == Target::Button(button),
        );
        return;
    }
    if area.height < 3 {
        frame.render_widget(
            Paragraph::new(super::components::clip_text(label, area.width as usize))
                .alignment(Alignment::Center)
                .style(if active || app.focus == Target::Button(button) {
                    theme::selected()
                } else {
                    theme::dim()
                }),
            area,
        );
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
        super::components::tab_button(frame, rect, &label, active, focused);
    }
}

pub(crate) fn pane(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme::BORDER).bg(theme::BG))
        .title(title.to_string())
        .title_style(theme::dim())
        .style(theme::text())
}

/// Like `pane` but highlights the border and appends ` · focused` when `focused` is true.
pub fn focused_pane(title: &str, focused: bool) -> Block<'static> {
    if focused {
        let focused_title = format!("{} • focused ", title.trim_end_matches(' '));
        Block::default()
            .borders(Borders::ALL)
            .border_style(theme::accent())
            .title(focused_title)
            .title_style(theme::accent())
            .style(theme::text())
    } else {
        pane(title)
    }
}

/// Render a `see more` footer button inside `pane_area` when the content
/// (measured in wrapped lines) overflows the inner viewport. Returns the
/// adjusted inner content area (shrunk by 1 row for the footer reservation).
///
/// `scroll` is the current line offset. `total_lines` is the total wrapped
/// line count. `page_size` is stored so the button handler can page by it.
/// The button is registered with the layout registry; clicking or Enter pages
/// down one viewport.
pub fn draw_see_more(
    frame: &mut Frame,
    app: &App,
    pane_area: Rect,
    scroll: u16,
    total_lines: usize,
    button: ButtonId,
    page_slot: usize,
) -> Rect {
    let inner = inset(pane_area);
    let inner_h = inner.height as usize;
    let has_more = total_lines.saturating_sub(scroll as usize) > inner_h.saturating_sub(1);
    if !has_more || inner_h < 3 {
        return pane_area;
    }
    // Store page size (inner height minus 1 overlap line) for the handler.
    let page = inner_h.saturating_sub(1).max(1) as u16;
    let mut pages = app.see_more_pages.get();
    pages[page_slot] = page;
    app.see_more_pages.set(pages);
    // Reserve the last row of the pane area for the footer.
    let footer_rect = Rect {
        x: pane_area.x,
        y: pane_area.y + pane_area.height.saturating_sub(1),
        width: pane_area.width,
        height: 1,
    };
    draw_button(frame, app, button, " see more ", footer_rect);
    // Return the content area with the footer row reserved.
    Rect {
        height: pane_area.height.saturating_sub(1),
        ..pane_area
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    app.layout.borrow_mut().clear();
    app.layout.borrow_mut().view_stamp = geometry_stamp(app);
    let area = frame.area();
    frame.render_widget(Paragraph::new("").style(theme::text()), area);
    let layout = chrome(area, app);
    if layout.header.height > 0 {
        draw_header(frame, app, &layout);
    }
    if layout.tab_strip.height > 0 {
        draw_tab_strip(frame, app, layout.tab_strip);
    }
    if (area.width < 40 || area.height < 12) && app.module != Some(ModuleId::System) {
        frame.render_widget(
            Paragraph::new(
                "Resize terminal to at least 40×12\nCtrl+K commands · ? help · Ctrl+Q quit",
            )
            .style(theme::dim())
            .wrap(Wrap { trim: false }),
            layout.body,
        );
        frame.render_widget(footer_line(app), layout.footer);
        if app.overlay != Overlay::None {
            draw_overlay(frame, app);
        }
        return;
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
        Some(ModuleId::Logs) => super::logs::draw(frame, app, layout.body),
        Some(ModuleId::Jobs) => {
            super::jobs::draw(frame, app, layout.body, app.job_source().is_some())
        }
    }
    if layout.composer.height > 0 {
        let areas = recon_composer_areas(layout.composer);
        draw_composer(frame, app, &areas, false);
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
    if app.module.is_none() {
        draw_tab_strip(frame, app, layout.header);
        let activity = if app.jobs.counts.active > 0 {
            format!("◌ {} active", app.jobs.counts.active)
        } else {
            String::new()
        };
        if !activity.is_empty() {
            let act_len = activity.chars().count() as u16;
            if layout.header.width > act_len + 2 {
                let act_rect = Rect {
                    x: layout.header.x + layout.header.width.saturating_sub(act_len + 1),
                    y: layout.header.y,
                    width: act_len + 1,
                    height: 1,
                };
                frame.render_widget(
                    Paragraph::new(Span::styled(activity, theme::accent())),
                    act_rect,
                );
            }
        }
        return;
    }
    let mut spans = Vec::new();
    let mut used = 0u16;
    for (module, rect) in header_tabs(layout.header, app.module) {
        used = used.max(rect.x + rect.width - layout.header.x);
        let label = match module {
            None => "[Home]",
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
        spans.push(Span::styled(
            if module.is_none() {
                format!("{label} > ")
            } else {
                format!("{label} ")
            },
            style,
        ));
    }
    let detail = header_detail(app);
    let activity = if app.jobs.counts.active > 0 {
        format!("◌ {} active", app.jobs.counts.active)
    } else {
        String::new()
    };
    let room = layout.header.width.saturating_sub(used.saturating_add(1)) as usize;
    let detail_room = room.saturating_sub(activity.chars().count().saturating_add(2));
    if detail_room > 4 && !detail.is_empty() {
        spans.push(Span::styled(
            format!("> {}", fit(&detail, detail_room.saturating_sub(2))),
            theme::dim(),
        ));
    }
    if !activity.is_empty() && room >= activity.chars().count().saturating_add(2) {
        let content_width: usize = spans.iter().map(|span| span.content.chars().count()).sum();
        let pad = (layout.header.width as usize)
            .saturating_sub(content_width.saturating_add(activity.chars().count()));
        spans.push(Span::raw(" ".repeat(pad)));
        spans.push(Span::styled(activity, theme::accent()));
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(theme::text()),
        layout.header,
    );
}

pub(crate) struct TabStripItem {
    pub target: Target,
    pub close_target: Option<Target>,
    pub rect: Rect,
    pub close_rect: Option<Rect>,
    pub label: String,
    pub active: bool,
    pub focused: bool,
    pub close_focused: bool,
    pub indicator: Option<&'static str>,
}

pub(crate) fn tab_strip_layout(area: Rect, app: &App) -> Vec<TabStripItem> {
    if area.height == 0 || area.width < 10 {
        return Vec::new();
    }
    let mut items = Vec::new();
    let mut x = area.x;

    // 1. Home Tab (permanent, first, not closable)
    let home_w = 8.min(area.width);
    let home_active = app.module.is_none() && app.tab_sel == 0;
    let home_focused = app.focus == Target::Tab(0);
    items.push(TabStripItem {
        target: Target::Tab(0),
        close_target: None,
        rect: Rect {
            x,
            y: area.y,
            width: home_w,
            height: 1,
        },
        close_rect: None,
        label: "Home".into(),
        active: home_active,
        focused: home_focused,
        close_focused: false,
        indicator: None,
    });
    x = x.saturating_add(home_w);

    // 2. Open investigation tabs with overflow
    let plus_w = 4u16;
    let overflow_reserve = 9u16;
    let total_tabs = app.tab_ids.len();
    let avail = area.width.saturating_sub(x - area.x + plus_w);
    let tab_w = 20u16.min(avail);

    if total_tabs > 0 && tab_w >= 10 {
        let max_visible = (avail as usize) / (tab_w as usize);
        let (start, end, show_overflow) = if total_tabs <= max_visible {
            (0, total_tabs, false)
        } else {
            let visible_cap = (avail.saturating_sub(overflow_reserve) as usize) / (tab_w as usize);
            let visible_cap = visible_cap.max(1);
            let active_idx = if app.tab_sel > 0 { app.tab_sel - 1 } else { 0 };
            let start = if active_idx >= visible_cap {
                active_idx + 1 - visible_cap
            } else {
                0
            };
            let end = (start + visible_cap).min(total_tabs);
            (start, end, true)
        };

        for idx in start..end {
            let thread_id = &app.tab_ids[idx];
            let visual_idx = idx + 1;
            let title = app
                .threads
                .iter()
                .find(|t| &t.id == thread_id)
                .map(|t| t.title.as_str())
                .unwrap_or("Investigation");
            let is_active = (app.module == Some(ModuleId::Recon) || app.module.is_none())
                && app.tab_sel == visual_idx;
            let is_focused = app.focus == Target::Tab(visual_idx);
            let is_close_focused = app.focus == Target::TabClose(visual_idx);

            let indicator = if app.running_thread(thread_id) {
                Some(super::anim::spinner())
            } else if app.tab_unreads.contains(thread_id) {
                Some("●")
            } else if app
                .thread_states
                .get(thread_id)
                .is_some_and(|s| s == "failed" || s == "interrupted")
            {
                Some("×")
            } else {
                None
            };

            let this_tab_w = tab_w.min(area.width.saturating_sub(x - area.x + plus_w));
            if this_tab_w < 6 {
                break;
            }
            let close_w = 2u16;
            let body_w = this_tab_w.saturating_sub(close_w);

            items.push(TabStripItem {
                target: Target::Tab(visual_idx),
                close_target: Some(Target::TabClose(visual_idx)),
                rect: Rect {
                    x,
                    y: area.y,
                    width: body_w,
                    height: 1,
                },
                close_rect: Some(Rect {
                    x: x.saturating_add(body_w),
                    y: area.y,
                    width: close_w,
                    height: 1,
                }),
                label: title.into(),
                active: is_active,
                focused: is_focused,
                close_focused: is_close_focused,
                indicator,
            });
            x = x.saturating_add(this_tab_w);
        }

        // 3. Overflow indicator
        if show_overflow {
            let hidden_count = total_tabs.saturating_sub(end - start);
            let of_w = overflow_reserve.min(area.width.saturating_sub(x - area.x + plus_w));
            if of_w >= 4 {
                items.push(TabStripItem {
                    target: Target::TabOverflow,
                    close_target: None,
                    rect: Rect {
                        x,
                        y: area.y,
                        width: of_w,
                        height: 1,
                    },
                    close_rect: None,
                    label: format!("›› ({hidden_count})"),
                    active: false,
                    focused: app.focus == Target::TabOverflow,
                    close_focused: false,
                    indicator: None,
                });
                x = x.saturating_add(of_w);
            }
        }
    }

    // 4. Plus action [+]
    if area.width.saturating_sub(x - area.x) >= plus_w {
        items.push(TabStripItem {
            target: Target::TabPlus,
            close_target: None,
            rect: Rect {
                x,
                y: area.y,
                width: plus_w,
                height: 1,
            },
            close_rect: None,
            label: "+".into(),
            active: false,
            focused: app.focus == Target::TabPlus,
            close_focused: false,
            indicator: None,
        });
    }

    items
}

pub(crate) fn draw_tab_strip(frame: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let items = tab_strip_layout(area, app);
    for item in items {
        app.layout.borrow_mut().register(item.target, item.rect);
        if let (Some(close_rect), Some(close_target)) = (item.close_rect, item.close_target) {
            app.layout.borrow_mut().register(close_target, close_rect);
        }
        let style = if item.focused {
            theme::selected()
        } else if item.active {
            theme::accent().add_modifier(Modifier::BOLD)
        } else {
            theme::dim()
        };

        let ind_str = item.indicator.map(|i| format!("{i} ")).unwrap_or_default();
        let display_label = if item.rect.width > 4 {
            let room = (item.rect.width as usize).saturating_sub(ind_str.chars().count() + 2);
            format!(" [{}{}]", ind_str, fit(&item.label, room))
        } else {
            format!(" [{}]", item.label)
        };

        frame.render_widget(
            Paragraph::new(Span::styled(display_label, style)),
            item.rect,
        );

        if let (Some(close_rect), Some(_)) = (item.close_rect, item.close_target) {
            let close_style = if item.close_focused {
                theme::selected()
            } else {
                theme::dim()
            };
            frame.render_widget(Paragraph::new(Span::styled("× ", close_style)), close_rect);
        }
    }
}

pub(crate) fn tab_strip_hit(app: &App, area: Rect, x: u16, y: u16) -> Option<Target> {
    for item in tab_strip_layout(area, app) {
        if let (Some(close_target), Some(close_rect)) = (item.close_target, item.close_rect) {
            if contains(close_rect, x, y) {
                return Some(close_target);
            }
        }
        if contains(item.rect, x, y) {
            return Some(item.target);
        }
    }
    None
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
            BrainListMode::Topology => "topology".into(),
            BrainListMode::Graph => {
                if let Some(title) = super::app::structural_title(app) {
                    title.trim().into()
                } else if app.detail_claim() {
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
            format!("{} · Overview / System / Configs", app.profile.tab.label())
        }
        Some(ModuleId::Jobs) => format!("{} active", app.jobs.counts.active),
        Some(ModuleId::Logs) => {
            let errors = app.error_count();
            if errors == 0 {
                "events".into()
            } else {
                super::logs::count(errors as i64, "error")
            }
        }
    }
}

fn footer_line(app: &App) -> Paragraph<'static> {
    let keys = if matches!(app.overlay, Overlay::Palette) {
        "type to filter · ↑↓ · Enter run · Esc close"
    } else if matches!(app.overlay, Overlay::AddFallback) {
        "←→ provider · type to filter · ↑↓ · Enter add · Esc close"
    } else if let Overlay::Choice(_) = app.overlay {
        "↑↓ choose · Enter select · Esc close"
    } else if app.overlay != Overlay::None {
        "Esc close · Ctrl+U/D scroll"
    } else {
        match (app.module, app.focus) {
            (None, _) => "↑↓ open · 1–9 apps · Ctrl+K · ? help",
            (Some(ModuleId::Intel), _) if app.intel_page == IntelPage::Briefing => {
                "↑↓ scroll · Related / Reader · Esc bulletin"
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
            ) => "↑↓ select · ←→ fold · o source · Enter toggle · Tab prompt",
            (Some(ModuleId::Recon), _) if !app.recon_chat => {
                "↑↓ open · Ctrl+N new · Ctrl+K · Esc home"
            }
            (Some(ModuleId::Recon), _) => "Tab next · Enter · Ctrl+K · Esc list",
            (Some(ModuleId::Brain), _) if app.brain_list_mode == BrainListMode::Graph => {
                "Tab section · ↑↓ select · Enter open related · Esc back · Ctrl+K"
            }
            (Some(ModuleId::Atlas), _) if app.atlas_news || app.atlas_page == AtlasPage::Live => {
                "Tab next · Enter · Ctrl+K · Esc history"
            }
            (Some(ModuleId::Logs), _) if app.logs.back_to_job.is_some() => {
                "↑↓ event · Enter fold · f follow · o job · Esc back to job"
            }
            (Some(ModuleId::Logs), _) => "↑↓ event · Enter fold · f follow · o job · Esc home",
            (Some(ModuleId::Jobs), _) => {
                "↑↓ job · Enter detail · l logs · r retry · c stop · s status · Esc home"
            }
            (Some(ModuleId::System), _)
                if app.profile.tab == super::profile::SystemTab::Configs =>
            {
                "Tab focus · Verify → Save · Enter JSON newline · Esc discard"
            }
            (Some(ModuleId::System), _) if app.profile.tab == super::profile::SystemTab::System => {
                "Tab focus · Host / Paths · r refresh hardware · x Configs"
            }
            (Some(ModuleId::System), _) => {
                "Tab focus · ↑↓ row · Enter detail · v chart/table · f filters · r refresh · ? help"
            }
            _ => "Tab next · 1–9 apps · Enter · Ctrl+K · Esc home",
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
    } else if app.module == Some(ModuleId::Osint) {
        if let Some(tool) = osint::registry().get(app.tool_sel) {
            parts.push(tool.name.to_string());
        }
    } else if app.module == Some(ModuleId::Logs) {
        let errors = app.error_count();
        if errors > 0 {
            parts.push(super::logs::count(errors as i64, "error"));
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
    let height = ((matches.len() * 2).saturating_sub(1) as u16 + 3)
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
        if index > 0 {
            lines.push(Line::raw(""));
        }
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
    ("jobs", "open Jobs"),
    ("logs", "open Logs"),
    ("tools", "open Tools (alias osint)"),
    ("models", "open Models (alias providers)"),
    ("system", "open System"),
    ("profile", "open Profile"),
    ("palette", "command palette"),
];

pub(crate) struct HomeComposerAreas {
    pub input: Rect,
    pub metadata: Rect,
    pub send: Rect,
    pub guidance: Rect,
    pub box_rect: Rect,
}

pub(crate) fn home_composer_areas(area: Rect) -> HomeComposerAreas {
    let metrics = home_layout_metrics(area);

    let box_rect = Rect {
        x: metrics.composer_x,
        y: metrics.box_y,
        width: metrics.composer_width,
        height: metrics.box_h,
    };

    let input_h = if metrics.box_h >= 3 { 2 } else { 1 };
    let input_rect = Rect {
        x: metrics.composer_x,
        y: metrics.box_y,
        width: metrics.composer_width,
        height: input_h,
    };

    let meta_y = metrics
        .box_y
        .saturating_add(metrics.box_h.saturating_sub(1));
    let send_w = 14.min(metrics.composer_width / 3).max(10);
    let meta_w = metrics.composer_width.saturating_sub(send_w);
    let metadata_rect = Rect {
        x: metrics.composer_x,
        y: meta_y,
        width: if metrics.composer_width >= 60 {
            metrics.composer_width
        } else {
            meta_w
        },
        height: 1,
    };
    let send_rect = Rect {
        x: metrics
            .composer_x
            .saturating_add(metrics.composer_width.saturating_sub(send_w)),
        y: meta_y,
        width: send_w,
        height: 1,
    };

    let guidance_rect = Rect {
        x: metrics.composer_x,
        y: metrics.guidance_y,
        width: metrics.composer_width,
        height: 1,
    };

    HomeComposerAreas {
        input: input_rect,
        metadata: metadata_rect,
        send: send_rect,
        guidance: guidance_rect,
        box_rect,
    }
}

pub(crate) fn recon_composer_areas(area: Rect) -> HomeComposerAreas {
    let composer_width = area.width;
    let composer_x = area.x;

    let box_h = area.height.max(2);
    let box_rect = Rect {
        x: composer_x,
        y: area.y,
        width: composer_width,
        height: box_h,
    };

    let input_h = box_h.saturating_sub(1).max(1);
    let input_rect = Rect {
        x: composer_x,
        y: area.y,
        width: composer_width,
        height: input_h,
    };

    let meta_y = area.y.saturating_add(box_h.saturating_sub(1));
    let send_w = 14.min(composer_width / 3).max(10);
    let meta_w = composer_width.saturating_sub(send_w);
    let metadata_rect = Rect {
        x: composer_x,
        y: meta_y,
        width: if composer_width >= 60 {
            composer_width
        } else {
            meta_w
        },
        height: 1,
    };
    let send_rect = Rect {
        x: composer_x.saturating_add(composer_width.saturating_sub(send_w)),
        y: meta_y,
        width: send_w,
        height: 1,
    };

    let guidance_rect = Rect {
        x: composer_x,
        y: area.y.saturating_add(box_h),
        width: 0,
        height: 0,
    };

    HomeComposerAreas {
        input: input_rect,
        metadata: metadata_rect,
        send: send_rect,
        guidance: guidance_rect,
        box_rect,
    }
}

pub(crate) fn cursor_blink_visible() -> bool {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| (d.as_millis() / 500) % 2 == 0)
        .unwrap_or(true)
}

fn composer_prompt_text<'a>(
    app: &'a App,
    is_focused: bool,
    blink_on: bool,
    is_home: bool,
) -> Text<'a> {
    let input = app.input.as_str();
    let cursor = app.cursor;
    let placeholder = if is_home {
        format!(
            "Ask anything... e.g. \"{}\"",
            app.recon_tip_text().unwrap_or_default()
        )
    } else {
        "Ask a follow-up... e.g. \"Dig deeper into the latest findings\"".to_string()
    };

    if input.is_empty() {
        if is_focused {
            if blink_on {
                Text::from(Line::from(vec![
                    Span::styled("█ ", theme::card_accent()),
                    Span::styled(placeholder, theme::card_dim()),
                ]))
            } else {
                Text::from(Line::from(vec![
                    Span::styled("  ", theme::card_dim()),
                    Span::styled(placeholder, theme::card_dim()),
                ]))
            }
        } else {
            Text::from(Line::from(vec![Span::styled(
                placeholder,
                theme::card_dim(),
            )]))
        }
    } else if !input.contains('\n') {
        let cursor_pos = input
            .char_indices()
            .nth(cursor)
            .map_or(input.len(), |(offset, _)| offset);
        let (before, after) = input.split_at(cursor_pos);
        let mut spans = Vec::new();
        if !before.is_empty() {
            spans.push(Span::styled(before.to_string(), theme::card_text()));
        }
        if is_focused {
            if let Some(ch) = after.chars().next() {
                let mut ch_buf = [0; 4];
                let ch_str = ch.encode_utf8(&mut ch_buf);
                if blink_on {
                    spans.push(Span::styled(
                        ch_str.to_string(),
                        Style::default()
                            .bg(theme::ACCENT)
                            .fg(theme::BG)
                            .add_modifier(Modifier::BOLD),
                    ));
                } else {
                    spans.push(Span::styled(ch_str.to_string(), theme::card_text()));
                }
                let rest = &after[ch.len_utf8()..];
                if !rest.is_empty() {
                    spans.push(Span::styled(rest.to_string(), theme::card_text()));
                }
            } else if blink_on {
                spans.push(Span::styled("█", theme::card_accent()));
            } else {
                spans.push(Span::styled(" ", theme::card_text()));
            }
        } else if !after.is_empty() {
            spans.push(Span::styled(after.to_string(), theme::card_text()));
        }
        Text::from(Line::from(spans))
    } else {
        let cursor_pos = input
            .char_indices()
            .nth(cursor)
            .map_or(input.len(), |(offset, _)| offset);
        let mut lines = Vec::new();
        let mut current_pos = 0;
        for line in input.split('\n') {
            let line_len = line.len();
            let line_end = current_pos + line_len;
            if is_focused && cursor_pos >= current_pos && cursor_pos <= line_end {
                let offset_in_line = cursor_pos - current_pos;
                let (before, after) = line.split_at(offset_in_line);
                let mut spans = Vec::new();
                if !before.is_empty() {
                    spans.push(Span::styled(before.to_string(), theme::card_text()));
                }
                if let Some(ch) = after.chars().next() {
                    let mut ch_buf = [0; 4];
                    let ch_str = ch.encode_utf8(&mut ch_buf);
                    if blink_on {
                        spans.push(Span::styled(
                            ch_str.to_string(),
                            Style::default()
                                .bg(theme::ACCENT)
                                .fg(theme::BG)
                                .add_modifier(Modifier::BOLD),
                        ));
                    } else {
                        spans.push(Span::styled(ch_str.to_string(), theme::card_text()));
                    }
                    let rest = &after[ch.len_utf8()..];
                    if !rest.is_empty() {
                        spans.push(Span::styled(rest.to_string(), theme::card_text()));
                    }
                } else if blink_on {
                    spans.push(Span::styled("█", theme::card_accent()));
                } else {
                    spans.push(Span::styled(" ", theme::card_text()));
                }
                lines.push(Line::from(spans));
            } else {
                lines.push(Line::from(Span::styled(
                    line.to_string(),
                    theme::card_text(),
                )));
            }
            current_pos = line_end + 1;
        }
        Text::from(lines)
    }
}

pub(crate) fn draw_composer(
    frame: &mut Frame,
    app: &App,
    areas: &HomeComposerAreas,
    is_home: bool,
) {
    app.layout
        .borrow_mut()
        .register(Target::Field(FieldId::Composer), areas.input);
    app.layout
        .borrow_mut()
        .register(Target::Button(ButtonId::Send), areas.send);
    let screen_h = frame.area().height;
    if areas.box_rect.y < screen_h {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme::SURFACE)),
            areas.box_rect,
        );

        let stripe_style = Style::default().fg(theme::ACCENT).bg(theme::SURFACE);
        for row in 0..areas.box_rect.height {
            let row_y = areas.box_rect.y + row;
            if row_y < screen_h {
                frame.render_widget(
                    Paragraph::new("▌").style(stripe_style),
                    Rect {
                        x: areas.box_rect.x,
                        y: row_y,
                        width: 1,
                        height: 1,
                    },
                );
            }
        }

        let prompt_rect = Rect {
            x: areas.box_rect.x.saturating_add(2),
            y: areas.box_rect.y,
            width: areas.box_rect.width.saturating_sub(4),
            height: areas.input.height,
        };
        let is_focused = app.focus == Target::Field(FieldId::Composer);
        let blink_on = cursor_blink_visible();
        let prompt_text = composer_prompt_text(app, is_focused, blink_on, is_home);
        let alignment = if is_home {
            Alignment::Center
        } else {
            Alignment::Left
        };
        frame.render_widget(
            Paragraph::new(prompt_text)
                .alignment(alignment)
                .wrap(Wrap { trim: false }),
            prompt_rect,
        );

        let meta_y = areas.box_rect.y + areas.box_rect.height.saturating_sub(1);
        if meta_y < screen_h {
            let recon_m = if app.recon_model.is_empty() {
                "default"
            } else {
                &app.recon_model
            };
            let synth_m = if app.synthesis_model.is_empty() {
                "default"
            } else {
                &app.synthesis_model
            };
            let mut spans = vec![
                Span::styled("Recon", theme::card_accent().add_modifier(Modifier::BOLD)),
                Span::styled(" · ", theme::card_dim()),
                Span::styled(
                    recon_m.to_string(),
                    theme::card_text().add_modifier(Modifier::BOLD),
                ),
                Span::styled(" · ", theme::card_dim()),
                Span::styled(
                    format!("Synthesis: {synth_m}"),
                    Style::default().fg(theme::WARN).bg(theme::SURFACE),
                ),
            ];
            if !is_home {
                spans.push(Span::styled(" · ", theme::card_dim()));
                spans.push(Span::styled(
                    "Enter send · Shift+Enter newline · /commands · Tab transcript · Esc list",
                    theme::card_dim(),
                ));
            }
            let meta_line = Line::from(spans);
            frame.render_widget(
                Paragraph::new(meta_line).alignment(alignment),
                areas.metadata,
            );

            let send_style = if app.focus == Target::Button(ButtonId::Send) {
                theme::selected()
            } else {
                theme::card_dim()
            };
            let action_label = if is_home {
                "[ Start ↵ ]"
            } else {
                "[ Send ↵ ]"
            };
            let send_alignment = if is_home {
                Alignment::Right
            } else {
                Alignment::Left
            };
            frame.render_widget(
                Paragraph::new(action_label)
                    .alignment(send_alignment)
                    .style(send_style),
                areas.send,
            );
        }
    }
    if areas.guidance.y < screen_h {
        let guidance_text = if is_home {
            "Enter to start · Shift+Enter / Ctrl+J for newline"
        } else {
            "" // Already merged into composer footer
        };
        if !guidance_text.is_empty() {
            frame.render_widget(
                Paragraph::new(guidance_text)
                    .alignment(Alignment::Center)
                    .style(theme::dim()),
                areas.guidance,
            );
        }
    }
}

fn draw_home(frame: &mut Frame, app: &App, area: Rect) {
    if area.height < 32 {
        let metrics = home_layout_metrics(area);
        let room = super::components::ListGeometry::capacity_with_gap(
            area.bottom().saturating_sub(metrics.apps_y) as usize,
            1,
            0,
        )
        .max(1);
        let start = app.scrolls.home as usize;
        for (index, module) in ModuleId::ALL
            .iter()
            .enumerate()
            .skip(start * 2)
            .take(room * 2)
        {
            let local = index - start * 2;
            let width = area.width / 2;
            let rect = Rect::new(
                area.x + (local % 2) as u16 * width,
                metrics.apps_y + (local / 2) as u16,
                width,
                1,
            );
            if rect.bottom() > area.bottom() {
                continue;
            }
            app.layout.borrow_mut().register(Target::App(index), rect);
            frame.render_widget(
                Paragraph::new(format!("{} {}", index + 1, module.title())).style(
                    if index == app.launcher_sel {
                        theme::selected()
                    } else {
                        theme::text()
                    },
                ),
                rect,
            );
        }
        draw_composer(frame, app, &home_composer_areas(area), true);
        return;
    }
    let rows = home_rows(area, app.error_count());
    let column = rows
        .iter()
        .filter(|row| !row.center)
        .map(home_row_width)
        .max()
        .unwrap_or(0) as u16;
    let column = column.min(area.width);
    let left = area.x + area.width.saturating_sub(column) / 2;
    for row in &rows {
        if row.y >= area.y.saturating_add(area.height) {
            break;
        }
        let line = home_line_text(row, app.launcher_sel);
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
        if let Some(index) = row.target {
            app.layout.borrow_mut().register(Target::App(index), rect);
        }
        frame.render_widget(
            Paragraph::new(line.unwrap()).alignment(if row.center {
                Alignment::Center
            } else {
                Alignment::Left
            }),
            rect,
        );
    }

    // Home composer
    let areas = home_composer_areas(area);
    draw_composer(frame, app, &areas, true);
    if app.input.starts_with('/') && app.focus == Target::Field(FieldId::Composer) {
        draw_slash_hint(frame, app, area);
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
    for index in 0..room.min(
        app.threads
            .len()
            .saturating_sub(app.scrolls.threads as usize),
    ) {
        app.layout.borrow_mut().register(
            Target::Thread(app.scrolls.threads as usize + index),
            Rect::new(
                list.x + 1,
                list.y + 1 + index as u16 * 2,
                list.width.saturating_sub(2),
                1,
            ),
        );
    }
    frame.render_widget(
        List::new(super::components::spaced(items, || ListItem::new("")))
            .block(pane(" investigations ")),
        list,
    );
    let thread_buttons = button_areas(actions, 1);
    draw_button(
        frame,
        app,
        ButtonId::DeleteThread,
        "Delete",
        thread_buttons[0],
    );
}

pub(crate) fn recon_context_buttons(area: Rect, app: &App) -> Vec<(ButtonId, Rect, &'static str)> {
    let mut defs = vec![(ButtonId::RetryInsights, recall_label(app))];
    if app.can_resume_recon() {
        defs.push((ButtonId::ResumeRun, "Resume"));
    }
    if app.can_stop_recon() {
        defs.push((ButtonId::CancelRun, "Stop"));
    }
    defs.push((
        ButtonId::ToggleInvestigation,
        if app.recon_context_enabled {
            "Hide details"
        } else {
            "More details"
        },
    ));

    let count = defs.len() as u16;
    if count == 0 || area.width < 4 || area.height < count * 2 {
        return Vec::new();
    }
    let btn_h = if area.height >= count * 3 + 4 { 3 } else { 2 };
    let total_h = count * btn_h;
    let start_y = area.y + area.height.saturating_sub(total_h);
    let btn_x = area.x.saturating_add(1);
    let btn_w = area.width.saturating_sub(1);

    defs.into_iter()
        .enumerate()
        .map(|(i, (id, label))| {
            let rect = Rect {
                x: btn_x,
                y: start_y + (i as u16) * btn_h,
                width: btn_w,
                height: btn_h,
            };
            (id, rect, label)
        })
        .collect()
}

fn draw_recon_chat(frame: &mut Frame, app: &App, area: Rect) {
    let (transcript, context, bottom_actions) = recon_chat_areas(app, area);
    if app.recon_context_enabled && app.screen.width < 110 {
        pane_tabs(
            frame,
            app,
            Rect::new(area.x, area.y, area.width, area.height.min(1)),
            &["Transcript", "Investigation"],
        );
    }
    if transcript.width > 0 && transcript.height > 0 {
        frame.render_widget(pane(" transcript "), transcript);
        draw_transcript(frame, app, transcript);
        if !app.chat_follow && transcript.height > 2 {
            let unread = chat_max(app).saturating_sub(app.scrolls.chat);
            let rect = Rect::new(
                transcript.x + 1,
                transcript.bottom() - 1,
                transcript.width.saturating_sub(2),
                1,
            );
            draw_button(
                frame,
                app,
                ButtonId::JumpLatest,
                &format!("Jump to latest · {unread} unread rows · End"),
                rect,
            );
        }
    }
    if let Some(context) = context {
        draw_recon_context(frame, app, context);
    } else if let Some(run_actions) = bottom_actions {
        let mut buttons = vec![(ButtonId::RetryInsights, recall_label(app))];
        if app.can_resume_recon() {
            buttons.push((ButtonId::ResumeRun, "Resume"));
        }
        if app.can_stop_recon() {
            buttons.push((ButtonId::CancelRun, "Stop"));
        }
        // On narrow terminals the investigation panel is hidden; offer an explicit toggle.
        let toggle_label = if app.recon_context_enabled {
            "Hide details"
        } else {
            "More details"
        };
        buttons.push((ButtonId::ToggleInvestigation, toggle_label));
        let count = buttons.len();
        let run_buttons = button_areas(run_actions, count);
        for (i, (id, label)) in buttons.into_iter().enumerate() {
            draw_button(frame, app, id, label, run_buttons[i]);
        }
    }
}

fn recon_context_lines(app: &App, width: u16) -> Vec<Line<'static>> {
    let mut lines = vec![String::new()];
    if let Some(thread) = app
        .threads
        .iter()
        .find(|thread| Some(&thread.id) == app.selected_thread.as_ref())
    {
        // Full title, wrapped by the Paragraph widget — no character clipping.
        lines.push(thread.title.clone());
    }
    if !app.recon_stage.is_empty() {
        lines.push(format!("Stage · {}", app.recon_stage));
    }
    lines.push(String::new());
    if let Some(run) = app.runs.last() {
        if let Some(plan) = run
            .plan_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Plan>(raw).ok())
        {
            lines.push(format!("Directives · {}", plan.directives.len()));
            // Show all directives; wrapping is handled by the Paragraph widget.
            for directive in plan.directives.iter() {
                if lines.last().is_some_and(|line: &String| !line.is_empty()) {
                    lines.push(String::new());
                }
                lines.push(format!(
                    "{}  {}",
                    directive.id.to_uppercase(),
                    directive.goal
                ));
            }
        }
    }
    lines.push(String::new());
    lines.push(format!("Evidence calls · {}", app.calls.len()));
    let memories: usize = app.answer_memories.values().map(Vec::len).sum();
    lines.push(String::new());
    lines.push(format!("▼ Memories used ({memories})"));
    for mem_list in app.answer_memories.values() {
        for mem in mem_list {
            if lines.last().is_some_and(|line: &String| !line.is_empty()) {
                lines.push(String::new());
            }
            // Full memory text, wrapped by the Paragraph widget.
            lines.push(format!("  • {}", mem.text));
        }
    }
    super::components::measured_lines(lines.into_iter().map(Line::raw).collect(), width)
}

pub(crate) fn shift_recon_context(app: &mut App, delta: i32) {
    let (_, Some(area), _) = recon_chat_areas(app, chrome(app.screen, app).body) else {
        return;
    };
    let inner = inset(area);
    let room = recon_context_buttons(area, app)
        .first()
        .map_or(inner.height, |(_, rect, _)| rect.y.saturating_sub(inner.y));
    let max = recon_context_lines(app, inner.width)
        .len()
        .saturating_sub(room as usize)
        .min(u16::MAX as usize) as u16;
    nudge(&mut app.scrolls.recon_context, delta, max);
}

fn draw_recon_context(frame: &mut Frame, app: &App, area: Rect) {
    let focused = app.focus == Target::ReconContext;
    let title = if focused {
        " Investigation · focused "
    } else {
        " Investigation "
    };
    let border_style = if focused {
        theme::accent()
    } else {
        theme::dim()
    };
    frame.render_widget(
        Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(border_style),
        area,
    );
    // Register the panel as a focus target using its full area.
    app.layout.borrow_mut().register(Target::ReconContext, area);
    let inner = inset(area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let buttons = recon_context_buttons(area, app);
    let info_h = if let Some((_, first_rect, _)) = buttons.first() {
        first_rect.y.saturating_sub(inner.y)
    } else {
        inner.height
    };
    let info_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: info_h.min(inner.height),
    };

    let lines = recon_context_lines(app, info_area.width);
    let scroll_offset = app.scrolls.recon_context.min(
        lines
            .len()
            .saturating_sub(info_area.height as usize)
            .min(u16::MAX as usize) as u16,
    );
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme::dim())
            .scroll((scroll_offset, 0)),
        info_area,
    );

    for (id, rect, label) in buttons {
        draw_button(frame, app, id, label, rect);
    }
}

fn draw_transcript(frame: &mut Frame, app: &App, _area: Rect) {
    app.layout.borrow_mut().register(Target::Transcript, _area);
    for spot in chat_spots(app) {
        app.layout.borrow_mut().register(spot.target, spot.rect);
    }
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
    for (offset, (_index, row)) in rows.iter().enumerate().skip(scroll).enumerate() {
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
    if face == RowFace::User {
        spans.push(Span::styled("▎ ", theme::accent().bg(theme::SURFACE)));
        used = 2;
    }
    for piece in pieces {
        let mut style = tone_style(piece.tone);
        if emphasize {
            style = style.add_modifier(Modifier::BOLD);
        }
        style = face_background(style, face);
        used += super::components::text_width(&piece.text);
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
    let layout = osint_areas(app, area, slot.is_some());
    let search = layout.search;
    let list = layout.list;
    let detail = layout.detail;
    let input = layout.input;
    let actions = layout.actions;
    draw_field(frame, app, FieldId::OsintSearch, " Tools ", search);
    if area.width < 68 || area.height < 26 {
        pane_tabs(
            frame,
            app,
            Rect::new(area.x, area.y + FIELD_H, area.width, 1),
            &["Catalog", "Documentation", "Test"],
        );
    }
    let rows = super::tool_catalog::rows(&app.osint_search, &app.tool_expanded);
    let start = app.scrolls.tools as usize;
    let items: Vec<_> = rows
        .iter()
        .skip(start)
        .take(list_room(list.height))
        .enumerate()
        .map(|(row, (target, label))| {
            let rect = Rect::new(
                list.x + 1,
                list.y + 1 + row as u16 * 2,
                list.width.saturating_sub(2),
                1,
            );
            app.layout.borrow_mut().register(*target, rect);
            ListItem::new(super::components::clip_text(label, rect.width as usize)).style(
                if app.focus == *target || *target == Target::Tool(app.tool_sel) {
                    theme::selected()
                } else {
                    theme::text()
                },
            )
        })
        .collect();
    frame.render_widget(
        List::new(super::components::spaced(items, || ListItem::new(""))).block(pane(" Catalog ")),
        list,
    );
    let response_text = app
        .osint_result
        .as_ref()
        .map(|(id, result)| {
            let body = if app.osint_raw {
                result.raw.clone()
            } else {
                serde_json::to_string_pretty(&result.observations).unwrap_or_default()
            };
            format!(
                "Run {id} · {} · {}\nSource: {}\n{}\n{body}",
                result.status,
                result.retrieved_at,
                result.source_url,
                result.error.as_deref().unwrap_or("")
            )
        })
        .unwrap_or_else(|| "No test response yet · enter JSON and Run".into());
    let response_inner = super::components::analytics_card(
        frame,
        layout.response,
        " Test response ",
        app.focus == Target::OsintResponse,
    );
    let response_lines =
        super::components::measured_lines(vec![Line::raw(response_text)], response_inner.width);
    let start = app.response_scroll.min(
        response_lines
            .len()
            .saturating_sub(response_inner.height as usize),
    );
    frame.render_widget(
        Paragraph::new(
            response_lines
                .into_iter()
                .skip(start)
                .take(response_inner.height as usize)
                .collect::<Vec<_>>(),
        ),
        response_inner,
    );
    app.layout
        .borrow_mut()
        .register(Target::OsintResponse, layout.response);
    let (desc, tool_id) = if let Some(tool) = osint::registry().get(app.tool_sel) {
        let docs = if tool.documentation.is_empty() {
            "Documentation unavailable"
        } else {
            tool.documentation
        };
        let dataset_status = if app.dataset_refresh_running.is_some() {
            "\nDataset refresh running · see Jobs".to_owned()
        } else {
            app.tool_dataset_status.clone()
        };
        let body = format!(
            "{}\n{} · {} · {}\n\nDocumentation\n{docs}\n\n{}\n\nInputs: {}\nExample: {}\n\nPolicy: {}\nTimeout: {}s · Cache: {}s{}",
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
            tool.description,
            tool.inputs.join(", "),
            tool.example_input(),
            tool.restrictions,
            tool.timeout_seconds,
            tool.cache_seconds,
            dataset_status
        );
        (body, tool.id)
    } else {
        ("Select a tool".into(), "")
    };
    let inner_w = inset(detail).width.max(1) as usize;
    let desc_lines = wrapped_line_count(&desc, inner_w);
    let detail = draw_see_more(
        frame,
        app,
        detail,
        app.scrolls.detail,
        desc_lines,
        ButtonId::SeeMoreDetail,
        0,
    );
    frame.render_widget(
        Paragraph::new(desc)
            .style(theme::text())
            .block(focused_pane(
                " Documentation ",
                app.focus == Target::OsintDetail,
            ))
            .scroll((app.scrolls.detail, 0))
            .wrap(Wrap { trim: true }),
        detail,
    );
    app.layout
        .borrow_mut()
        .register(Target::OsintDetail, detail);
    if let Some(slot) = slot {
        let parts = split_horizontal(layout.key, [Constraint::Min(8), Constraint::Length(16)]);
        draw_field(frame, app, slot.field, " API key ", parts[0]);
        draw_button(frame, app, slot.button, "Save keys", parts[1]);
        draw_field(frame, app, slot.fallback, " Fallback ", layout.fallback);
    }
    draw_field(frame, app, FieldId::OsintInput, " Input JSON ", input);
    let button_list = osint_buttons(
        if tool_id.is_empty() {
            None
        } else {
            Some(tool_id)
        },
        app.dataset_refresh_running.as_deref(),
    );
    let buttons = button_areas(actions, button_list.len());
    for (index, (button, label)) in button_list.into_iter().enumerate() {
        draw_button(frame, app, button, label, buttons[index]);
    }
}

fn draw_brain(frame: &mut Frame, app: &App, area: Rect) {
    if app.brain_list_mode == BrainListMode::Topology {
        super::topology::draw_topology(frame, app, area);
        return;
    }
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
    draw_field(frame, app, FieldId::BrainQuery, " Find ", layout.query);
    let room =
        super::components::ListGeometry::capacity(layout.list.height.saturating_sub(2) as usize, 2);
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
                    if memory.id.starts_with("actor-group:") {
                        "Actor"
                    } else {
                        "Claim"
                    },
                    memory.text
                ),
                inner_w,
            );
            let source = if memory.id.starts_with("actor-group:") {
                fit(&format!("  {}", memory.category), inner_w)
            } else {
                fit(
                    &format!(
                        "  {} / {}",
                        memory.source.app, memory.source.conversation_id
                    ),
                    inner_w,
                )
            };
            let selected = matches!(app.focus, Target::Memory(_)) && index == app.memory_sel;
            ListItem::new(format!("{title}\n{source}")).style(if selected {
                theme::selected()
            } else {
                theme::text()
            })
        })
        .collect::<Vec<_>>();
    for row in 0..room.min(
        app.memories
            .len()
            .saturating_sub(app.scrolls.memories as usize),
    ) {
        app.layout.borrow_mut().register(
            Target::Memory(app.scrolls.memories as usize + row),
            Rect::new(
                layout.list.x + 1,
                layout.list.y + 1 + row as u16 * 3,
                layout.list.width.saturating_sub(2),
                2,
            ),
        );
    }
    let title = memory_list_title(app);
    if items.is_empty() && app.memory_error.is_none() && !app.memories_loaded {
        let block = pane(&title);
        let inner = block.inner(layout.list);
        frame.render_widget(block, layout.list);
        super::anim::draw_centered(frame.buffer_mut(), inner, "Loading memories", theme::dim());
    } else if items.is_empty() {
        let (text, style) = memory_list_note(app);
        frame.render_widget(
            Paragraph::new(text)
                .style(style)
                .block(pane(&title))
                .wrap(Wrap { trim: true }),
            layout.list,
        );
    } else {
        frame.render_widget(
            List::new(super::components::spaced(items, || ListItem::new(""))).block(pane(&title)),
            layout.list,
        );
    }
    let action_buttons = button_areas(layout.actions, 4);
    draw_button(
        frame,
        app,
        ButtonId::CreateMemory,
        "Create",
        action_buttons[0],
    );
    if app
        .memories
        .get(app.memory_sel)
        .is_some_and(|m| m.id.starts_with("actor-group:"))
    {
        for (index, label) in [(1, "Pin"), (2, "Delete")] {
            frame.render_widget(
                Paragraph::new(label)
                    .style(theme::dim())
                    .block(pane(" unavailable ")),
                action_buttons[index],
            );
        }
    } else {
        draw_button(frame, app, ButtonId::Pin, "Pin", action_buttons[1]);
        draw_button(frame, app, ButtonId::Delete, "Delete", action_buttons[2]);
    }
    draw_button(
        frame,
        app,
        ButtonId::BrainKind,
        &format!("Kind: {}", super::app::brain_kind_label(app.brain_kind)),
        action_buttons[3],
    );
    let recalled = if matches!(app.focus, Target::Memory(_)) {
        if let Some(insight) = &app.selected_insight {
            memory_anchors_text(insight)
        } else {
            "Find filters saved memory. Select an investigation insight to read its anchors.".into()
        }
    } else {
        "Find filters saved memory. Select an investigation insight to read its anchors.".into()
    };
    let recall_content = recalled.clone();
    let recall_w = inset(layout.recall).width.max(1) as usize;
    let recall_lines = wrapped_line_count(&recall_content, recall_w);
    let recall_area = draw_see_more(
        frame,
        app,
        layout.recall,
        app.scrolls.recall,
        recall_lines,
        ButtonId::SeeMoreRecall,
        1,
    );
    frame.render_widget(
        Paragraph::new(recalled)
            .style(theme::text())
            .block(focused_pane(" anchors ", app.focus == Target::BrainRecall))
            .scroll((app.scrolls.recall, 0))
            .wrap(Wrap { trim: true }),
        recall_area,
    );
    app.layout
        .borrow_mut()
        .register(Target::BrainRecall, recall_area);
}

/// Memory list title: read failure first, then an active Find filter.
fn memory_list_title(app: &App) -> String {
    if app.memory_error.is_some() && !app.memories.is_empty() {
        return " memories · read failed · showing last loaded list ".into();
    }
    if app.brain_refreshing() && app.memories_loaded {
        return " memories · refreshing ".into();
    }
    let kind = app
        .brain_kind
        .map(|k| format!(" · {}", super::app::brain_kind_label(Some(k))))
        .unwrap_or_default();
    if !app.brain_query.trim().is_empty() && app.memories_loaded {
        return format!(
            " memories{kind} · Find active · {} of {} ",
            app.memories.len(),
            app.memory_total
        );
    }
    if app.brain_kind.is_some() {
        return format!(" memories{kind} · {} ", app.memories.len());
    }
    " memories ".into()
}

/// Empty-list message: loading, read failure, no memories, or no Find matches.
pub(crate) fn memory_list_note(app: &App) -> (String, ratatui::style::Style) {
    if let Some(err) = &app.memory_error {
        return (
            format!("Could not read memories: {err}\nThe error is in Logs. The list reloads on the next change."),
            theme::error(),
        );
    }
    if !app.memories_loaded {
        return (super::anim::loading("Loading memories"), theme::dim());
    }
    let query = app.brain_query.trim();
    if !query.is_empty() {
        return (
            format!(
                "No memories match \"{query}\". Find is still active; clear it to see all {}.",
                app.memory_total
            ),
            theme::dim(),
        );
    }
    (
        "No memories yet. Create one, or analyze an article in Intel or investigate with Recon."
            .into(),
        theme::dim(),
    )
}

pub(crate) fn memory_anchors_text(insight: &argos_osint_core::recon::InsightView) -> String {
    let mut lines = vec![insight.presentation()];
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
    } else if let Some(url) = source
        .source_url
        .as_deref()
        .filter(|url| !url.trim().is_empty())
    {
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

fn model_catalog_lines(labels: impl IntoIterator<Item = String>, width: u16) -> Vec<Line<'static>> {
    let records = labels
        .into_iter()
        .map(|label| super::components::measured_lines(vec![Line::raw(label)], width));
    super::components::spaced(records, || vec![Line::raw("")])
        .into_iter()
        .flatten()
        .collect()
}

pub(crate) fn provider_catalog_scroll_max(app: &App) -> u16 {
    let (provider, filter) = match app.provider_page {
        ProviderPage::Google => ("google", &app.google_model_filter),
        ProviderPage::Nvidia => ("nvidia", &app.nvidia_model_filter),
        ProviderPage::OpenRouter => ("openrouter", &app.router_model_filter),
        ProviderPage::Defaults => return 0,
    };
    let body = chrome(app.screen, app).body;
    let router = router_areas(provider_areas(body)[1]);
    let catalog = split_vertical(
        router[5],
        [
            Constraint::Length(FIELD_H),
            Constraint::Length(ACTION_H),
            Constraint::Min(0),
        ],
    );
    let inner = inset(catalog[2]);
    let models = app
        .catalog_cache
        .get(provider)
        .map(Vec::as_slice)
        .unwrap_or_else(|| {
            if app.catalog_for == provider {
                app.model_catalog.as_slice()
            } else {
                &[]
            }
        });
    let query = filter.trim().to_ascii_lowercase();
    model_catalog_lines(
        models
            .iter()
            .filter(|model| {
                query.is_empty()
                    || model.id.to_ascii_lowercase().contains(&query)
                    || model.name.to_ascii_lowercase().contains(&query)
            })
            .map(|model| format!("{} · {}", model.name, model.id)),
        inner.width,
    )
    .len()
    .saturating_sub(inner.height as usize)
    .min(u16::MAX as usize) as u16
}

fn draw_providers(frame: &mut Frame, app: &App, area: Rect) {
    let rows = provider_areas(area);
    for (page, rect) in ProviderPage::ALL
        .into_iter()
        .zip(button_areas(rows[0], ProviderPage::ALL.len()))
    {
        app.layout
            .borrow_mut()
            .register(Target::ProviderTab(page), rect);
    }
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
        ProviderPage::Google | ProviderPage::Nvidia | ProviderPage::OpenRouter => {
            let (
                status,
                key_field,
                _key_val,
                btn_save,
                btn_verify,
                btn_advanced,
                advanced,
                ep_field,
                _ep_val,
                title,
            ) = match app.provider_page {
                ProviderPage::Google => (
                    app.google_status.as_str(),
                    FieldId::GoogleKey,
                    &app.google_key,
                    ButtonId::GoogleSave,
                    ButtonId::GoogleVerify,
                    ButtonId::GoogleAdvanced,
                    app.google_advanced,
                    FieldId::GoogleEndpoint,
                    &app.google_endpoint,
                    " google ",
                ),
                ProviderPage::Nvidia => (
                    app.nvidia_status.as_str(),
                    FieldId::NvidiaKey,
                    &app.nvidia_key,
                    ButtonId::NvidiaSave,
                    ButtonId::NvidiaVerify,
                    ButtonId::NvidiaAdvanced,
                    app.nvidia_advanced,
                    FieldId::NvidiaEndpoint,
                    &app.nvidia_endpoint,
                    " nvidia ",
                ),
                ProviderPage::OpenRouter => (
                    app.router_status.as_str(),
                    FieldId::RouterKey,
                    &app.router_key,
                    ButtonId::RouterSave,
                    ButtonId::RouterVerify,
                    ButtonId::RouterAdvanced,
                    app.router_advanced,
                    FieldId::RouterEndpoint,
                    &app.router_endpoint,
                    " openrouter ",
                ),
                _ => unreachable!(),
            };

            let router = router_areas(rows[1]);
            frame.render_widget(
                Paragraph::new(status)
                    .style(theme::accent())
                    .block(pane(title))
                    .wrap(Wrap { trim: true }),
                router[0],
            );
            draw_field(frame, app, key_field, " API key ", router[1]);
            let buttons = button_areas(router[2], 2);
            draw_button(frame, app, btn_save, "Save", buttons[0]);
            draw_button(frame, app, btn_verify, "Verify", buttons[1]);
            draw_button(
                frame,
                app,
                btn_advanced,
                if advanced {
                    "Hide endpoint"
                } else {
                    "Show endpoint"
                },
                router[3],
            );
            if advanced {
                draw_field(frame, app, ep_field, " HTTPS endpoint ", router[4]);
            }
            let catalog_area = split_vertical(
                router[5],
                [
                    Constraint::Length(FIELD_H),
                    Constraint::Length(ACTION_H),
                    Constraint::Min(0),
                ],
            );
            let (filter_field, filter_text, provider_id) = match app.provider_page {
                ProviderPage::Google => (
                    FieldId::GoogleModelFilter,
                    &app.google_model_filter,
                    "google",
                ),
                ProviderPage::Nvidia => (
                    FieldId::NvidiaModelFilter,
                    &app.nvidia_model_filter,
                    "nvidia",
                ),
                ProviderPage::OpenRouter => (
                    FieldId::RouterModelFilter,
                    &app.router_model_filter,
                    "openrouter",
                ),
                ProviderPage::Defaults => unreachable!(),
            };
            draw_field(frame, app, filter_field, " Filter models ", catalog_area[0]);
            draw_button(
                frame,
                app,
                ButtonId::RefreshModels,
                "Refresh models",
                catalog_area[1],
            );
            let catalog = app
                .catalog_cache
                .get(provider_id)
                .cloned()
                .or_else(|| (app.catalog_for == provider_id).then(|| app.model_catalog.clone()))
                .unwrap_or_default();
            let query = filter_text.trim().to_ascii_lowercase();
            let filtered: Vec<_> = catalog
                .iter()
                .filter(|model| {
                    query.is_empty()
                        || model.id.to_ascii_lowercase().contains(&query)
                        || model.name.to_ascii_lowercase().contains(&query)
                })
                .cloned()
                .collect();
            let title = if catalog.is_empty() {
                if app.catalog_for == provider_id {
                    " Models · empty catalog ".to_string()
                } else {
                    " Models · not loaded ".to_string()
                }
            } else if filtered.is_empty() {
                format!(" Models · 0 of {} ", catalog.len())
            } else {
                format!(" Models · {} of {} ", filtered.len(), catalog.len())
            };
            let lines: Vec<Line> = if catalog.is_empty() {
                vec![Line::from(
                    "Catalog not loaded. Refresh models after saving a key.",
                )]
            } else if filtered.is_empty() {
                vec![Line::from("No models match this filter.")]
            } else {
                model_catalog_lines(
                    filtered
                        .iter()
                        .map(|model| format!("{} · {}", model.name, model.id)),
                    inset(catalog_area[2]).width,
                )
            };
            let list = draw_see_more(
                frame,
                app,
                catalog_area[2],
                app.scrolls.detail,
                lines.len(),
                ButtonId::SeeMoreDetail,
                0,
            );
            frame.render_widget(
                Paragraph::new(lines)
                    .block(focused_pane(
                        &title,
                        matches!(app.focus, Target::Button(ButtonId::SeeMoreDetail)),
                    ))
                    .scroll((app.scrolls.detail, 0))
                    .wrap(Wrap { trim: false }),
                list,
            );
        }
        ProviderPage::Defaults => {
            let models = model_areas(app, rows[1]);
            if rows[1].width < 100 || rows[1].height < 30 {
                pane_tabs(
                    frame,
                    app,
                    Rect::new(rows[1].x, rows[1].y, rows[1].width, 1),
                    &["Roles", "Assignment / fallbacks"],
                );
            }

            if rows[1].width >= 100 && rows[1].height >= 30 {
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
            } else if models[0].height > 0 {
                frame.render_widget(
                    Paragraph::new(app.defaults_role.label()).style(theme::accent()),
                    models[0],
                );
            }
            let role = app.defaults_role;
            let provider = role.provider_field();
            let model = role.model_field();
            let save = role.save_button();
            draw_field(frame, app, provider, " Provider ", models[1]);
            draw_field(frame, app, model, " Model ", models[2]);
            // Save the role default, or edit one Brief operation's override
            // with the same Provider/Model fields.
            let save_row = button_areas(models[3], 4);
            draw_button(frame, app, save, "Save default", save_row[0]);
            draw_button(
                frame,
                app,
                ButtonId::BriefOpCycle,
                &format!("Op: {} ▸", app.selected_brief_op().1),
                save_row[1],
            );
            draw_button(
                frame,
                app,
                ButtonId::BriefOpSave,
                "Save for op",
                save_row[2],
            );
            draw_button(frame, app, ButtonId::BriefOpReset, "Reset op", save_row[3]);
            draw_fallbacks_list(frame, app, models[4]);
            let actions = button_areas(models[5], 5);
            draw_button(
                frame,
                app,
                ButtonId::AddFallback,
                "Add fallback",
                actions[0],
            );
            draw_button(
                frame,
                app,
                ButtonId::DeleteFallback,
                "Delete selected",
                actions[1],
            );
            draw_button(frame, app, ButtonId::MoveFallbackUp, "Move up", actions[2]);
            draw_button(
                frame,
                app,
                ButtonId::MoveFallbackDown,
                "Move down",
                actions[3],
            );
            draw_button(
                frame,
                app,
                ButtonId::BriefFastPreset,
                if app.settings.defaults.fast_preset_applied() {
                    "Reset fast preset"
                } else {
                    "Apply fast preset"
                },
                actions[4],
            );
            let note = "Tried top to bottom after primary retries. Primary: 4 attempts (10s/20s/30s). Each fallback: 3 attempts (10s/20s).";
            if models[6].height >= 8 {
                // The note needs one row when it fits; give the rest to the
                // role details (operation evidence must stay visible).
                let note_rows = if (note.chars().count() as u16) <= models[6].width {
                    1
                } else {
                    3
                };
                let split = split_vertical(
                    models[6],
                    [Constraint::Length(note_rows), Constraint::Min(0)],
                );
                frame.render_widget(
                    Paragraph::new(note)
                        .style(theme::dim())
                        .wrap(Wrap { trim: true }),
                    split[0],
                );
                super::model_roles::draw_model_roles_view(frame, app, split[1]);
            } else {
                frame.render_widget(
                    Paragraph::new(note)
                        .style(theme::dim())
                        .wrap(Wrap { trim: true }),
                    models[6],
                );
            }
        }
    }
}

fn draw_fallbacks_list(frame: &mut Frame, app: &App, area: Rect) {
    let focused = matches!(
        app.focus,
        Target::Button(ButtonId::FallbackItem(_) | ButtonId::AddFallback)
    );
    let title = if focused {
        " Fallbacks · focused "
    } else {
        " Fallbacks "
    };
    frame.render_widget(pane(title), area);
    let inner = inset(area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let fallbacks = app
        .settings
        .defaults
        .role(app.defaults_role.role_key())
        .map(|a| a.fallbacks.as_slice())
        .unwrap_or(&[]);
    if fallbacks.is_empty() {
        frame.render_widget(
            Paragraph::new("No fallback models configured")
                .style(theme::dim())
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }
    let mut lines = Vec::new();
    let start = app.scrolls.fallbacks as usize;
    for (i, route) in
        fallbacks
            .iter()
            .enumerate()
            .skip(start)
            .take(super::components::ListGeometry::capacity(
                inner.height as usize,
                1,
            ))
    {
        if i > start {
            lines.push(Line::raw(""));
        }
        app.layout.borrow_mut().register(
            Target::Button(ButtonId::FallbackItem(i)),
            Rect {
                x: inner.x,
                y: inner.y.saturating_add((i - start) as u16 * 2),
                width: inner.width,
                height: 1,
            },
        );
        let mut label = format!("{}. {}", i + 1, route.label());
        let secret = provider::account_secret(&app.auth, &route.provider);
        if secret.api_key.as_deref().unwrap_or("").trim().is_empty() {
            label.push_str(" · missing key");
        }
        let style = if i == app.fallback_sel {
            theme::selected()
        } else {
            theme::text()
        };
        lines.push(Line::from(Span::styled(
            super::components::clip_text(&label, inner.width as usize),
            style,
        )));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn atlas_live_areas(app: &App, area: Rect) -> (Rect, Rect, Rect, Rect) {
    if area.width < 100 || area.height < 30 {
        let actions = Rect::new(area.x, area.y, area.width, ACTION_H.min(area.height));
        let content = Rect::new(
            area.x,
            area.y + ACTION_H + 1,
            area.width,
            area.height.saturating_sub(ACTION_H + 1),
        );
        return match pane_page(app) % 3 {
            0 => (actions, content, Rect::default(), Rect::default()),
            1 => (actions, Rect::default(), content, Rect::default()),
            _ => (actions, Rect::default(), Rect::default(), content),
        };
    }
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
    if on {
        "recall: on"
    } else {
        "recall: off"
    }
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
    super::components::ListGeometry::capacity_with_gap(
        intel_bulletin_areas(chrome(app.screen, app).body)
            .4
            .height
            .saturating_sub(2) as usize,
        1,
        0,
    )
    .max(1)
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

fn intel_briefing_areas(app: &App, area: Rect) -> (Rect, Rect, Rect) {
    if area.width < 132 || area.height < 28 {
        let content = compact_content(area);
        if area.width >= 100 && area.height >= 28 && pane_page(app) < 2 {
            let columns = split_horizontal(
                content,
                [Constraint::Percentage(35), Constraint::Percentage(65)],
            );
            return (columns[0], columns[1], Rect::default());
        }
        return match pane_page(app) % 3 {
            0 => (content, Rect::default(), Rect::default()),
            1 => (Rect::default(), content, Rect::default()),
            _ => (Rect::default(), Rect::default(), content),
        };
    }
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
    for (index, rect) in button_areas(tabs, INTEL_CATEGORIES.len())
        .into_iter()
        .enumerate()
    {
        app.layout
            .borrow_mut()
            .register(Target::IntelTab(index), rect);
    }
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
    let mut story_inner = story_block.inner(hero);
    frame.render_widget(story_block, hero);
    if story_inner.height > 1 {
        let visit = Rect::new(
            story_inner.x,
            story_inner.bottom() - 1,
            story_inner.width,
            1,
        );
        draw_button(
            frame,
            app,
            ButtonId::IntelVisit,
            "Visit article site",
            visit,
        );
        story_inner.height -= 1;
    }
    let (image, preview) = bulletin_image_preview(app, story_inner);
    draw_article_image(frame, app, image);
    story_inner = preview;
    let story_lines = intel_hero_lines(app, story_inner.width as usize);
    let lines = super::components::measured_lines(story_lines, story_inner.width);
    let start = app
        .intel_preview_scroll
        .min(lines.len().saturating_sub(story_inner.height as usize));
    // Center short previews; overflowing previews retain their full scroll area.
    let spare = story_inner
        .height
        .saturating_sub(lines.len().min(u16::MAX as usize) as u16);
    let text_area = Rect::new(
        story_inner.x,
        story_inner.y + spare / 2,
        story_inner.width,
        story_inner.height - spare / 2,
    );
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .skip(start)
                .take(story_inner.height as usize)
                .collect::<Vec<_>>(),
        ),
        text_area,
    );
    app.layout
        .borrow_mut()
        .register(Target::IntelPreview, story_inner);

    draw_field(frame, app, FieldId::IntelSearch, "filter", search);

    let list_block = pane(" secondary stories ");
    let list_inner = list_block.inner(list);
    frame.render_widget(list_block, list);
    if app.intel_articles.is_empty() {
        frame.render_widget(
            Paragraph::new(if app.intel_bulletin_loading() {
                super::anim::loading("Loading bulletin")
            } else {
                " No classified articles for this day. Run Atlas, or pick another day. ".into()
            })
            .style(theme::dim()),
            list_inner,
        );
        return;
    }
    let room = intel_list_room(app);
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
    for row in 0..room.min(
        app.intel_articles
            .len()
            .saturating_sub(app.scrolls.intel_list as usize),
    ) {
        app.layout.borrow_mut().register(
            Target::IntelArticle(app.scrolls.intel_list as usize + row),
            Rect::new(list_inner.x, list_inner.y + row as u16, list_inner.width, 1),
        );
    }
    frame.render_widget(List::new(items), list_inner);
}

fn bulletin_image_preview(app: &App, area: Rect) -> (Rect, Rect) {
    let visible = app
        .intel_articles
        .get(app.intel_sel)
        .is_some_and(|article| app.article_images.visible(&article.id, &article.image_url));
    if !visible || area.height < 3 {
        return (Rect::default(), area);
    }
    if area.width >= 100 {
        let width = (area.width * 32 / 100).min(40);
        (
            Rect::new(area.x, area.y, width, area.height),
            Rect::new(
                area.x + width + 1,
                area.y,
                area.width.saturating_sub(width + 1),
                area.height,
            ),
        )
    } else {
        let height = (area.height / 2).clamp(2, 8);
        (
            Rect::new(area.x, area.y, area.width, height),
            Rect::new(
                area.x,
                area.y + height + 1,
                area.width,
                area.height.saturating_sub(height + 1),
            ),
        )
    }
}

pub(crate) fn article_image_area(app: &App) -> Rect {
    if app.module != Some(ModuleId::Intel) || app.overlay != Overlay::None {
        return Rect::default();
    }
    let body = chrome(app.screen, app).body;
    if app.intel_page == IntelPage::Bulletin {
        let (_, _, hero, _, _) = intel_bulletin_areas(body);
        let mut inner = pane(" image ").inner(hero);
        inner.height = inner.height.saturating_sub(1);
        bulletin_image_preview(app, inner).0
    } else {
        let (_, center, _) = intel_briefing_areas(app, body);
        intel_center_layout(app, center)
            .and_then(|layout| intersect_abs(center, layout.image))
            .map_or(Rect::default(), |(rect, _)| rect)
    }
}

fn draw_article_image(frame: &mut Frame, app: &App, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if let Some(article) = app.intel_articles.get(app.intel_sel) {
        app.article_images
            .draw(frame, area, &article.id, &article.image_url);
        if !url::Url::parse(&article.url)
            .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some())
        {
            return;
        }
        app.layout
            .borrow_mut()
            .register(Target::Button(ButtonId::IntelVisit), area);
    }
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
                article.title.clone(),
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
    if app
        .status
        .starts_with(&format!("Could not open {}:", article.url))
    {
        lines.splice(
            0..0,
            wrap_text(&app.status, width.max(1))
                .into_iter()
                .map(|text| Line::styled(text, theme::accent())),
        );
    }
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

pub(crate) fn wrap_text(text: &str, width: usize) -> Vec<String> {
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
    let (left, center, right) = intel_briefing_areas(app, area);
    if area.width < 132 || area.height < 28 {
        pane_tabs(
            frame,
            app,
            Rect::new(area.x, area.y, area.width, 1),
            &["Extracted", "Reader", "Context / jobs"],
        );
    }
    app.layout
        .borrow_mut()
        .register(Target::IntelLeftColumn, left);

    frame.render_widget(Block::default().style(theme::text()), left);
    if let Some(layout) = intel_extracted_layout(app, left) {
        let detail = intel_insights_progress_detail(app);
        draw_clipped_md_pane(
            frame,
            left,
            layout.confidence,
            " confidence ",
            &layout.confidence_lines,
            0,
        );
        // 1. Actors remain visible throughout analytical refresh.
        draw_intel_topology_records(
            frame,
            app,
            left,
            layout.actors,
            &format!(" actors · {} ", layout.actor_count),
            &layout.actors_lines,
            &layout.actor_records,
        );
        // 2. claims
        if layout.claims_loading {
            draw_clipped_intel_loading(
                frame,
                left,
                layout.claims,
                " claims ",
                &layout.phases[0],
                detail.as_deref(),
            );
        } else {
            draw_intel_topology_records(
                frame,
                app,
                left,
                layout.claims,
                " claims ",
                &layout.claims_lines,
                &layout.claim_records,
            );
        }
        // 3. inferences
        if layout.inferences_loading {
            draw_clipped_intel_loading(
                frame,
                left,
                layout.inferences,
                " inferences ",
                &layout.phases[1],
                detail.as_deref(),
            );
        } else {
            draw_intel_topology_records(
                frame,
                app,
                left,
                layout.inferences,
                " inferences ",
                &layout.inferences_lines,
                &layout.inference_records,
            );
        }
        // 4. links
        if layout.links_loading {
            let phase = if !layout.phases[2].is_empty() {
                layout.phases[2].as_str()
            } else if intel_insights_loading(app) || intel_artifact_pending(app, "Links") {
                "Extracting links"
            } else {
                "Explaining links"
            };
            draw_clipped_intel_loading(
                frame,
                left,
                layout.links,
                " links ",
                phase,
                detail.as_deref(),
            );
        } else {
            draw_clipped_md_pane(frame, left, layout.links, " links ", &layout.links_lines, 0);
        }
    }

    // Side panes stay fixed; only the center stack scrolls.
    frame.render_widget(Block::default().style(theme::text()), center);
    if let Some(layout) = intel_center_layout(app, center) {
        // The brief pane owns the background: paint it before the image column.
        draw_intel_brief_article(frame, center, &layout);
        if let Some((rect, _)) = intersect_abs(center, layout.brief) {
            app.layout.borrow_mut().register(Target::IntelPreview, rect);
        }
        if let Some((rect, _)) = intersect_abs(center, layout.image) {
            draw_article_image(frame, app, rect);
        }
        draw_clipped_button(
            frame,
            app,
            center,
            intel_reader_actions(layout.reload).0,
            ButtonId::IntelVisit,
            "Visit article site",
        );
        draw_clipped_button(
            frame,
            app,
            center,
            intel_reader_actions(layout.reload).1,
            ButtonId::IntelBodyRefresh,
            "Reload article",
        );
        if intel_body_loading(app) {
            draw_intel_body_loading(frame, center, layout.full, app);
        } else {
            draw_clipped_md_pane(
                frame,
                center,
                layout.full,
                if matches!(
                    app.focus,
                    Target::IntelFullArticle | Target::Button(ButtonId::IntelBodyMore)
                ) {
                    " full article • focused "
                } else {
                    " full article "
                },
                &layout.full_lines[..layout.full_lines.len().min(
                    app.scrolls.intel_full.min(layout.full_scroll_max) as usize + layout.full_room,
                )],
                app.scrolls.intel_full.min(layout.full_scroll_max),
            );
        }
        if let Some((rect, _)) = intersect_abs(center, layout.full) {
            app.layout
                .borrow_mut()
                .register(Target::IntelFullArticle, rect);
        }
        if let Some(more) = layout.full_more {
            draw_clipped_button(
                frame,
                app,
                center,
                more,
                ButtonId::IntelBodyMore,
                "See more",
            );
        }
        draw_intel_related(frame, app, center, layout.related);
    }

    let right_rows = intel_briefing_right_rows(app, right);
    draw_intel_tags(frame, article, right_rows[0]);
    super::map::draw_country_mini_map(
        frame,
        right_rows[1],
        &article.country,
        article.temperature,
        "Country",
    );
    draw_intel_jobs_pane(frame, app, right_rows[2]);
}

/// Brief article section: the image occupies a left column beside the brief
/// prose, both vertically centered inside the bordered pane.
fn draw_intel_brief_article(frame: &mut Frame, viewport: Rect, layout: &IntelCenterLayout) {
    let Some((vis, _top_clip)) = intersect_abs(viewport, layout.brief) else {
        return;
    };
    frame.render_widget(Block::default().style(theme::text()), vis);
    stroke_clipped_box(frame, viewport, layout.brief, " brief article ");
    let Some((text_vis, skipped)) = intersect_abs(viewport, layout.brief_text) else {
        return;
    };
    let visible: Vec<Line> = layout
        .brief_lines
        .iter()
        .skip(skipped as usize)
        .take(text_vis.height as usize)
        .cloned()
        .collect();
    frame.render_widget(Paragraph::new(visible), text_vis);
}

#[derive(Clone, Copy)]
struct AbsRect {
    x: u16,
    y: i32,
    width: u16,
    height: u16,
}

fn intel_reader_actions(area: AbsRect) -> (AbsRect, AbsRect) {
    let first = area.width.saturating_mul(2) / 3;
    (
        AbsRect {
            width: first,
            ..area
        },
        AbsRect {
            x: area.x + first,
            width: area.width - first,
            ..area
        },
    )
}

struct IntelCenterLayout {
    image: AbsRect,
    brief: AbsRect,
    brief_text: AbsRect,
    reload: AbsRect,
    full: AbsRect,
    full_more: Option<AbsRect>,
    full_room: usize,
    related: AbsRect,
    brief_lines: Vec<Line<'static>>,
    full_lines: Vec<Line<'static>>,
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
    // Two borders, one readable row and the three-row See more action cannot overlap.
    let full_h = term_pct_height(app, 0.50, 8).min(viewport.height.saturating_sub(8).max(6));

    // The article image lives in a left column inside the brief article section;
    // the prose keeps the remaining columns and both are vertically centered.
    let gap: u16 = 1;
    let mut image_w: u16 = 0;
    let mut image_h: u16 = 0;
    if app.article_images.visible(&article.id, &article.image_url) {
        // ~35% of the reader inner width, bounded, but never squeeze the brief prose
        image_w = ((inner_w as u16).saturating_mul(35) / 100).clamp(14, 40);
        let text_room = (inner_w as u16).saturating_sub(24 + gap);
        if image_w > text_room {
            image_w = text_room;
        }
        if image_w >= 14 {
            // ~20% of the reader height, bounded; never taller than the box is wide
            // in pixel terms (terminal cells are ~1:2 pixels, so rows <= cols / 2)
            image_h = (viewport.height.saturating_mul(20) / 100)
                .clamp(5, 12)
                .min((image_w / 2).max(3));
        } else {
            // Too narrow: collapse, text keeps the full width.
            image_w = 0;
        }
    }
    let image_columns = if image_w > 0 { image_w + gap } else { 0 };
    let text_w = (inner_w as u16).saturating_sub(image_columns).max(1) as usize;

    let mut brief_lines = intel_brief_preview_lines(article, text_w);
    if app
        .status
        .starts_with(&format!("Could not open {}:", article.url))
    {
        brief_lines.splice(
            0..0,
            wrap_text(&app.status, text_w)
                .into_iter()
                .map(|text| Line::styled(text, theme::accent())),
        );
    }
    let brief_h = (brief_lines.len() as u16)
        .max(image_h)
        .saturating_add(2)
        .max(min_section);

    let full_lines = intel_brief_full_lines(app, inner_w);
    let full_inner = full_h.saturating_sub(2).max(1);
    let has_more = !intel_body_loading(app)
        && full_lines
            .len()
            .saturating_sub(app.scrolls.intel_full as usize)
            > full_inner as usize;
    let full_room = full_inner
        .saturating_sub(if has_more { ACTION_H } else { 0 })
        .max(1);
    let full_scroll_max = full_lines.len().saturating_sub(full_room as usize) as u16;

    // The focused pair fits together at wide/medium viewports, keeping the
    // Full Article header visible when Related is revealed.
    let related_h = viewport.height.saturating_sub(full_h).max(8);
    let total = brief_h
        .saturating_add(ACTION_H)
        .saturating_add(full_h)
        .saturating_add(related_h);
    let stack_scroll_max = total.saturating_sub(viewport.height);
    let scroll = app.scrolls.intel_brief.min(stack_scroll_max) as i32;

    let mut y = i32::from(viewport.y) - scroll;
    let brief = abs_rect(viewport.x, y, viewport.width, brief_h);
    // Image and prose sit inside the brief section, vertically centered.
    let brief_inner_h = brief_h.saturating_sub(2);
    let image_top = brief_inner_h.saturating_sub(image_h) / 2;
    let text_top =
        brief_inner_h.saturating_sub(brief_lines.len().min(brief_inner_h as usize) as u16) / 2;
    let image = abs_rect(
        viewport.x + 1,
        brief.y + 1 + i32::from(image_top),
        image_w,
        image_h,
    );
    let brief_text = abs_rect(
        viewport.x + 1 + image_columns,
        brief.y + 1 + i32::from(text_top),
        text_w as u16,
        brief_lines.len() as u16,
    );
    y += i32::from(brief_h);
    let reload = abs_rect(viewport.x, y, viewport.width, ACTION_H);
    y += i32::from(ACTION_H);
    let full = abs_rect(viewport.x, y, viewport.width, full_h);
    let full_more = has_more.then(|| {
        abs_rect(
            viewport.x + 1,
            y + i32::from(full_h.saturating_sub(ACTION_H + 1)),
            viewport.width.saturating_sub(2),
            ACTION_H,
        )
    });
    y += i32::from(full_h);
    let related = abs_rect(viewport.x, y, viewport.width, related_h);

    Some(IntelCenterLayout {
        image,
        brief,
        brief_text,
        reload,
        full,
        full_more,
        full_room: full_room as usize,
        related,
        brief_lines,
        full_lines,
        full_scroll_max,
        stack_scroll_max,
    })
}

pub fn intel_brief_scroll_max(app: &App) -> u16 {
    let body = chrome(app.screen, app).body;
    let (_left, center, _right) = intel_briefing_areas(app, body);
    intel_center_layout(app, center)
        .map(|layout| layout.stack_scroll_max)
        .unwrap_or(0)
}

pub(super) fn intel_full_scroll_max(app: &App) -> u16 {
    let body = chrome(app.screen, app).body;
    let (_left, center, _right) = intel_briefing_areas(app, body);
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
    let border = Style::default()
        .fg(if title.contains("• focused") {
            theme::ACCENT
        } else {
            theme::BORDER
        })
        .bg(theme::BG);
    let title_style = if title.contains("• focused") {
        theme::accent()
    } else {
        theme::dim()
    };
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
                    buf[(x, row)].set_char(ch).set_style(title_style);
                }
                x = x.saturating_add(1);
            }
        }
    }

    let bottom_visible = top_clip.saturating_add(vis.height) >= area.height;
    if bottom_visible {
        let row = bottom as u16;
        if bottom >= 0 && row >= viewport.y && row < viewport.y.saturating_add(viewport.height) {
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

fn intel_briefing_right_rows(app: &App, right: Rect) -> Vec<Rect> {
    // Map must be at least 30% of total terminal height.
    let map_floor = term_pct_height(app, 0.30, 8);
    let reserved = 3u16.saturating_add(4);
    let map_h = map_floor.min(right.height.saturating_sub(reserved)).max(8);
    split_vertical(
        right,
        [
            Constraint::Length(3),
            Constraint::Length(map_h),
            Constraint::Min(4),
        ],
    )
}

fn extracted_claims_markdown(items: &[intel_recon::ExtractedLine]) -> String {
    if items.is_empty() {
        return "_none_".to_string();
    }
    items
        .iter()
        .map(|item| match item.confidence {
            Some(score) => format!("- {} ({score:.2})", item.text),
            None => format!("- {}", item.text),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn extracted_inferences_markdown(items: &[intel_recon::ExtractedLine]) -> String {
    if items.is_empty() {
        return "_none_".to_string();
    }
    items
        .iter()
        .map(|item| match item.confidence {
            Some(score) => format!("- {} ({score:.2})", item.text),
            None => format!("- {}", item.text),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn extracted_actors_markdown(items: &[String]) -> String {
    if items.is_empty() {
        return "_none extracted_".to_string();
    }
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn extracted_links_markdown(items: &[String]) -> String {
    if items.is_empty() {
        return "_none_".to_string();
    }
    items
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn parse_extracted_md(text: &str, width: usize) -> Vec<Line<'static>> {
    super::markdown::markdown_lines(text, width.max(1))
        .into_iter()
        .map(md_line_to_line)
        .collect()
}

struct IntelTopologyRows {
    lines: Vec<Line<'static>>,
    records: Vec<(std::ops::Range<usize>, Target)>,
}

/// Measure complete records before clipping. Gaps retain their normal background
/// and never acquire the neighboring record's action.
fn intel_topology_rows(
    texts: impl IntoIterator<Item = String>,
    width: usize,
    target: fn(usize) -> Target,
) -> IntelTopologyRows {
    let measured: Vec<_> = texts
        .into_iter()
        .map(|text| {
            super::components::measured_lines(
                parse_extracted_md(&format!("- {text}"), width),
                width.clamp(1, u16::MAX as usize) as u16,
            )
        })
        .collect();
    let geometry = super::components::ListGeometry::new(measured.iter().map(Vec::len));
    let mut lines = Vec::with_capacity(geometry.extent);
    let mut records = Vec::with_capacity(measured.len());
    for (index, (range, record)) in geometry.items.into_iter().zip(measured).enumerate() {
        lines.resize(range.start, Line::raw(""));
        lines.extend(record);
        records.push((range, target(index)));
    }
    IntelTopologyRows { lines, records }
}

fn intel_stage_topology_rows(
    app: &App,
    operation: &str,
    texts: impl IntoIterator<Item = String>,
    width: usize,
    target: fn(usize) -> Target,
) -> IntelTopologyRows {
    let mut rows = intel_topology_rows(texts, width, target);
    let stage = app
        .intel_brief_stages
        .iter()
        .find(|s| s.operation == operation);
    let mut prefix = stage
        .filter(|s| s.state != "completed")
        .map(|s| parse_extracted_md(&format!("_{}_\n\n_Previous_", s.row), width))
        .unwrap_or_default();
    if !prefix.is_empty() {
        prefix.push(Line::raw(""));
    }
    let offset = prefix.len();
    for (range, _) in &mut rows.records {
        range.start += offset;
        range.end += offset;
    }
    prefix.extend(rows.lines);
    if let Some(withheld) = app
        .intel_brief_outputs
        .get(operation)
        .and_then(|out| out["withheld"].as_u64())
        .filter(|n| *n > 0)
    {
        prefix.push(Line::raw(""));
        prefix.extend(parse_extracted_md(
            &format!("_{withheld} withheld · not supported by cited text_"),
            width,
        ));
    }
    rows.lines = prefix;
    rows
}

fn draw_intel_topology_records(
    frame: &mut Frame,
    app: &App,
    viewport: Rect,
    area: AbsRect,
    title: &str,
    lines: &[Line<'static>],
    records: &[(std::ops::Range<usize>, Target)],
) {
    draw_clipped_md_pane(frame, viewport, area, title, lines, 0);
    for (range, target) in records {
        let record = abs_rect(
            area.x.saturating_add(1),
            area.y + 1 + range.start.min(i32::MAX as usize) as i32,
            area.width.saturating_sub(2),
            range.len().min(u16::MAX as usize) as u16,
        );
        if let Some((visible, clip)) = intersect_abs(viewport, record) {
            if app.focus == *target {
                let selected: Vec<_> = lines
                    .iter()
                    .skip(range.start + clip as usize)
                    .take(visible.height as usize)
                    .cloned()
                    .map(|mut line| {
                        line.style = theme::selected();
                        for span in &mut line.spans {
                            span.style = theme::selected();
                        }
                        line
                    })
                    .collect();
                frame.render_widget(Paragraph::new(selected).style(theme::selected()), visible);
            }
            app.layout.borrow_mut().register(*target, visible);
        }
    }
}

struct IntelExtractedLayout {
    confidence: AbsRect,
    claims: AbsRect,
    inferences: AbsRect,
    actors: AbsRect,
    links: AbsRect,

    claims_lines: Vec<Line<'static>>,
    confidence_lines: Vec<Line<'static>>,
    inferences_lines: Vec<Line<'static>>,
    actors_lines: Vec<Line<'static>>,
    claim_records: Vec<(std::ops::Range<usize>, Target)>,
    inference_records: Vec<(std::ops::Range<usize>, Target)>,
    actor_records: Vec<(std::ops::Range<usize>, Target)>,
    links_lines: Vec<Line<'static>>,
    claims_loading: bool,
    inferences_loading: bool,
    actor_count: usize,
    links_loading: bool,
    /// Loading labels (stage rows while a brief stage runs).
    phases: [String; 3],
    stack_scroll_max: u16,
}

fn intel_extracted_layout(app: &App, viewport: Rect) -> Option<IntelExtractedLayout> {
    let article = app.intel_articles.get(app.intel_sel)?;
    if viewport.width < 4 || viewport.height < 4 {
        return None;
    }
    let inner_w = viewport.width.saturating_sub(2).max(1) as usize;
    let confidence_lines = intel_confidence_lines(app, inner_w);
    let mut buckets = intel_recon::bucket_extracted_with_explanations(
        &app.intel_claims,
        &app.intel_relations,
        &app.intel_link_explanations,
    );

    let grounded = std::mem::take(&mut buckets.actors);
    buckets.actors = app
        .intel_actors
        .iter()
        .map(|actor| {
            format!(
                "{} · {} · {}",
                actor.display_name,
                actor.type_hint,
                if actor.mentions.is_empty() {
                    "Grounded in current source"
                } else {
                    "Mentioned in title/description"
                }
            )
        })
        .collect();
    for actor in grounded {
        if !app
            .intel_actors
            .iter()
            .any(|candidate| candidate.display_name.eq_ignore_ascii_case(&actor))
        {
            buckets
                .actors
                .push(format!("{actor} · grounded in current source"));
        }
    }
    let claims_md = extracted_claims_markdown(&buckets.facts);
    let inferences_md = if buckets.inferences.is_empty() {
        format!(
            "_{} · inferences withheld until evidence is ready_",
            app.intel_insight_lifecycle.replace('_', " ")
        )
    } else {
        extracted_inferences_markdown(&buckets.inferences)
    };
    // Streamed progression lives in Background jobs, not in content panes.
    let actors_md = if buckets.actors.is_empty() {
        format!("_NLP {}_", app.intel_actor_state)
    } else {
        extracted_actors_markdown(&buckets.actors)
    };
    let mut links_md = extracted_links_markdown(&buckets.links);
    if !buckets.links.is_empty()
        && app
            .intel_claims
            .iter()
            .any(|claim| claim.fingerprint.starts_with("v2:"))
        && app.intel_insight_lifecycle != "active"
    {
        links_md = format!(
            "_{} · retained assessed links_\n\n{links_md}",
            app.intel_insight_lifecycle.replace('_', " ")
        );
    }

    let mut claims_md = claims_md;
    let mut inferences_md = inferences_md;
    let mut phases = [
        "Extracting claims".to_string(),
        "Extracting inferences".to_string(),
        String::new(),
    ];
    let brief = !app.intel_brief_stages.is_empty();
    let mut brief_loading = [false; 3];
    if brief {
        for (n, (op, md)) in [
            ("intel_brief_claims", &mut claims_md),
            ("intel_brief_inferences", &mut inferences_md),
            ("intel_brief_links", &mut links_md),
        ]
        .into_iter()
        .enumerate()
        {
            let Some(stage) = app.intel_brief_stages.iter().find(|s| s.operation == op) else {
                continue;
            };
            if stage.state == "completed" {
                *md = brief_stage_markdown(app, op);
            } else if stage.active {
                brief_loading[n] = true;
                phases[n] = stage.row.clone();
            } else {
                // A section shows its own stage state; history is Previous.
                let previous =
                    !md.trim().is_empty() && !md.contains("_none_") && !md.starts_with('_');
                // A failed stage, or one blocked by a failed predecessor, names
                // the failure and how to recover; nothing is added otherwise.
                let head = if stage.detail.is_empty() {
                    format!("_{}_", stage.row)
                } else {
                    format!("_{}_\n{}", stage.row, stage.detail)
                };
                *md = if previous {
                    format!("{head}\n\n_Previous_\n\n{md}")
                } else {
                    head
                };
            }
        }
    }
    let mut claims_lines = parse_extracted_md(&claims_md, inner_w);
    let mut inferences_lines = parse_extracted_md(&inferences_md, inner_w);
    let mut actors_lines = parse_extracted_md(&actors_md, inner_w);
    let links_lines = parse_extracted_md(&links_md, inner_w);
    let insights_loading = intel_insights_loading(app);
    let (claims_loading, inferences_loading, links_loading) = if brief {
        (brief_loading[0], brief_loading[1], brief_loading[2])
    } else {
        (
            (insights_loading || intel_artifact_pending(app, "Claims")) && buckets.facts.is_empty(),
            (insights_loading || intel_artifact_pending(app, "Insights"))
                && buckets.inferences.is_empty(),
            insights_loading
                || (intel_artifact_pending(app, "Links") && buckets.links.is_empty())
                || app.intel_links_reviewing.contains(&article.id),
        )
    };

    let claim_entries = app.intel_topology_claim_entries();
    let mut claim_records = Vec::new();
    if !claims_loading && !claim_entries.is_empty() {
        let rows = intel_stage_topology_rows(
            app,
            "intel_brief_claims",
            claim_entries.into_iter().map(|(_, text)| text),
            inner_w,
            Target::IntelTopologyClaim,
        );
        claims_lines = rows.lines;
        claim_records = rows.records;
    }
    let inference_entries = app.intel_topology_inference_entries();
    let mut inference_records = Vec::new();
    if !inferences_loading && !inference_entries.is_empty() {
        let rows = intel_stage_topology_rows(
            app,
            "intel_brief_inferences",
            inference_entries.into_iter().map(|(_, text)| text),
            inner_w,
            Target::IntelTopologyInference,
        );
        inferences_lines = rows.lines;
        inference_records = rows.records;
    }
    let actor_entries = app.intel_topology_actor_entries();
    let actor_count = actor_entries.len();
    let mut actor_records = Vec::new();
    if !actor_entries.is_empty() {
        let rows = intel_topology_rows(
            actor_entries.into_iter().map(|(actor_ref, name)| {
                app.intel_actors
                    .iter()
                    .find(|actor| actor.id == actor_ref)
                    .map(|actor| {
                        format!(
                            "{name} · {} · {}",
                            actor.type_hint,
                            if actor.mentions.is_empty() {
                                "Grounded in current source"
                            } else {
                                "Mentioned in title/description"
                            }
                        )
                    })
                    .unwrap_or_else(|| format!("{name} · grounded in current source"))
            }),
            inner_w,
            Target::IntelTopologyActor,
        );
        actors_lines = rows.lines;
        actor_records = rows.records;
    }

    let section_h = |lines_count: usize, loading: bool| -> u16 {
        let natural = (lines_count as u16).saturating_add(2);
        if loading {
            natural.max(5)
        } else {
            natural.max(3)
        }
    };

    let claims_h = section_h(claims_lines.len(), claims_loading);
    let inferences_h = section_h(inferences_lines.len(), inferences_loading);
    let actors_h = section_h(actors_lines.len(), false);
    let links_h = section_h(links_lines.len(), links_loading);
    let confidence_h = section_h(confidence_lines.len(), false);
    let total = confidence_h
        .saturating_add(claims_h)
        .saturating_add(inferences_h)
        .saturating_add(actors_h)
        .saturating_add(links_h);
    let stack_scroll_max = total.saturating_sub(viewport.height);
    let scroll = app.scrolls.intel_extracted.min(stack_scroll_max) as i32;

    let mut y = i32::from(viewport.y) - scroll;
    let confidence = abs_rect(viewport.x, y, viewport.width, confidence_h);
    y += i32::from(confidence_h);
    let actors = abs_rect(viewport.x, y, viewport.width, actors_h);
    y += i32::from(actors_h);
    let claims = abs_rect(viewport.x, y, viewport.width, claims_h);
    y += i32::from(claims_h);
    let inferences = abs_rect(viewport.x, y, viewport.width, inferences_h);
    y += i32::from(inferences_h);
    let links = abs_rect(viewport.x, y, viewport.width, links_h);

    Some(IntelExtractedLayout {
        confidence,
        confidence_lines,
        claims,
        inferences,
        actors,
        links,
        claims_lines,
        inferences_lines,
        actors_lines,
        claim_records,
        inference_records,
        actor_records,
        links_lines,
        claims_loading,
        inferences_loading,
        actor_count,
        links_loading,
        phases,
        stack_scroll_max,
    })
}

pub fn reveal_intel_related(app: &mut App, index: usize) {
    let body = chrome(app.screen, app).body;
    let (_, center, _) = intel_briefing_areas(app, body);
    let Some(layout) = intel_center_layout(app, center) else {
        return;
    };
    let room = layout.related.height.saturating_sub(3).max(1) as usize;
    let rows = super::related_articles::rows(
        &app.intel_related,
        center.width.saturating_sub(2) as usize,
        false,
    );
    let top = rows
        .iter()
        .position(|(_, t)| *t == Some(Target::IntelRelatedArticle(index)))
        .unwrap_or(0);
    reveal_index(&mut app.intel_related.scroll, top, room);
    let y = layout.related.y;
    if y < i32::from(center.y) {
        app.scrolls.intel_brief = app
            .scrolls
            .intel_brief
            .saturating_sub((i32::from(center.y) - y) as u16);
    } else if y + i32::from(layout.related.height) > i32::from(center.y + center.height) {
        app.scrolls.intel_brief = app
            .scrolls
            .intel_brief
            .saturating_add(
                (y + i32::from(layout.related.height) - i32::from(center.y + center.height)) as u16,
            )
            .min(layout.stack_scroll_max);
    }
}

/// The same measured record ranges drive painting, pointer targets and keyboard
/// reveal, including rows currently outside the extracted-column viewport.
pub fn reveal_intel_topology_entry(app: &mut App, target: Target) {
    let body = chrome(app.screen, app).body;
    let (mut left, _, _) = intel_briefing_areas(app, body);
    if left.width == 0 {
        app.compact_pages[ModuleId::Intel.index()] = 0;
        left = intel_briefing_areas(app, body).0;
    }
    let Some(layout) = intel_extracted_layout(app, left) else {
        return;
    };
    let (area, records) = match target {
        Target::IntelTopologyClaim(_) => (layout.claims, &layout.claim_records),
        Target::IntelTopologyInference(_) => (layout.inferences, &layout.inference_records),
        Target::IntelTopologyActor(_) => (layout.actors, &layout.actor_records),
        _ => return,
    };
    let Some((range, _)) = records.iter().find(|(_, row_target)| *row_target == target) else {
        return;
    };
    let effective_offset = app.scrolls.intel_extracted.min(layout.stack_scroll_max);
    let start = (i64::from(area.y) - i64::from(left.y)
        + i64::from(effective_offset)
        + 1
        + range.start as i64)
        .max(0) as usize;
    let mut pane = super::components::ScrollPane {
        offset: effective_offset as usize,
    };
    pane.reveal(start, range.len(), left.height as usize);
    app.scrolls.intel_extracted = pane.offset.min(layout.stack_scroll_max as usize) as u16;
}

fn draw_intel_related(frame: &mut Frame, app: &App, viewport: Rect, area: AbsRect) {
    let focused = matches!(
        app.focus,
        Target::IntelRelatedArticle(_)
            | Target::Button(ButtonId::IntelRelatedMore | ButtonId::RelatedScope)
    );
    let title = if focused {
        " Related • focused "
    } else {
        " Related "
    };
    let rows = super::related_articles::rows(
        &app.intel_related,
        area.width.saturating_sub(2) as usize,
        false,
    );
    let more = app.intel_related.page.has_more;
    let room = area.height.saturating_sub(2 + u16::from(more));
    let lines = rows
        .iter()
        .take(app.intel_related.scroll as usize + room as usize)
        .map(|(line, _)| line.clone())
        .collect::<Vec<_>>();
    draw_clipped_md_pane(
        frame,
        viewport,
        area,
        title,
        &lines,
        app.intel_related.scroll,
    );
    for (offset, (_, target)) in rows
        .iter()
        .skip(app.intel_related.scroll as usize)
        .take(room as usize)
        .enumerate()
    {
        if let Some(target) = target {
            if let Some((rect, _)) = intersect_abs(
                viewport,
                abs_rect(
                    area.x + 1,
                    area.y + 1 + offset as i32,
                    area.width.saturating_sub(2),
                    1,
                ),
            ) {
                app.layout.borrow_mut().register(*target, rect);
            }
        }
    }
    if more {
        draw_clipped_button(
            frame,
            app,
            viewport,
            abs_rect(
                area.x + 1,
                area.y + i32::from(area.height) - 2,
                area.width.saturating_sub(2),
                1,
            ),
            ButtonId::IntelRelatedMore,
            "See more",
        );
    }
}

pub fn intel_extracted_scroll_max(app: &App) -> u16 {
    let body = chrome(app.screen, app).body;
    let (left, _center, _right) = intel_briefing_areas(app, body);
    intel_extracted_layout(app, left)
        .map(|layout| layout.stack_scroll_max)
        .unwrap_or(0)
}

fn draw_clipped_intel_loading(
    frame: &mut Frame,
    viewport: Rect,
    area: AbsRect,
    title: &str,
    phase: &str,
    detail: Option<&str>,
) {
    let Some((vis, _)) = intersect_abs(viewport, area) else {
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
    let Some((inner_vis, _)) = intersect_abs(viewport, inner) else {
        return;
    };
    // Shared centered loading animation; streamed progression is shown in
    // Background jobs instead of inside content panes.
    let _ = detail;
    frame.render_widget(Block::default().style(theme::text()), inner_vis);
    super::anim::draw_centered(frame.buffer_mut(), inner_vis, phase, theme::dim());
}

/// Streamed progress for one artifact (Claims / Insights / Links) of the
/// focused article's insight refresh.
pub(crate) fn intel_artifact_progress<'a>(
    app: &'a App,
    artifact: &str,
) -> Option<&'a argos_osint_core::investigation::insight_execution::ArtifactProgress> {
    app.intel_extraction_progress
        .as_ref()?
        .artifacts
        .iter()
        .find(|a| a.artifact == artifact)
}

/// True while the artifact's content has not been published yet.
pub(crate) fn intel_artifact_pending(app: &App, artifact: &str) -> bool {
    intel_artifact_progress(app, artifact).is_some_and(|a| {
        !(a.step.starts_with("Published")
            || a.step.starts_with("Indexed")
            || a.step.starts_with("Failed")
            || a.step.starts_with("Cancelled")
            || a.step.starts_with("Superseded"))
    })
}

fn draw_intel_jobs_pane(frame: &mut Frame, app: &App, area: Rect) {
    let block = pane(" jobs ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = intel_brief_task_lines(app);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// Focus-brief background task rows: full article fetch, insight re-extract, mode classify.
fn intel_brief_task_lines(app: &App) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let Some(article) = app.intel_articles.get(app.intel_sel) else {
        return lines;
    };

    lines.push(Line::from(Span::styled(
        "BACKGROUND",
        theme::accent().add_modifier(Modifier::BOLD),
    )));

    if !app.intel_brief_stages.is_empty() {
        brief_stage_lines(app, &mut lines);
    } else {
        legacy_brief_task_lines(app, article, &mut lines);
    }

    lines
}

/// Pre-chain rows (legacy refresh/body tracking) for articles without a
/// brief preparation chain.
fn legacy_brief_task_lines(
    app: &App,
    article: &argos_osint_core::store::AtlasArticleRow,
    lines: &mut Vec<Line<'static>>,
) {
    let _ = article;
    let body_running = intel_body_loading(app);
    let insights_running = intel_insights_loading(app);

    // Full article retrieval / refine.
    let (body_title, body_detail) = match intel_body_presentation(app) {
        IntelBodyPresentation::Retrieving => {
            let (label, detail) = intel_body_progress_lines(app);
            (label, detail)
        }
        IntelBodyPresentation::Content { warning, .. } => {
            let quality = if warning.is_some() {
                app.intel_body
                    .as_ref()
                    .map_or("ready", |body| body.quality.as_str())
            } else {
                "ready"
            };
            (format!("Full article · {quality}"), warning)
        }
        IntelBodyPresentation::Status(_) => ("Full article · unavailable".into(), None),
    };
    let body_style = if body_running {
        theme::accent()
    } else {
        theme::text()
    };
    lines.push(Line::from(Span::styled(
        body_title,
        body_style.add_modifier(Modifier::BOLD),
    )));
    if let Some(detail) = body_detail {
        lines.push(Line::from(Span::styled(
            format!("  {detail}"),
            theme::dim(),
        )));
    }

    // Per-artifact streamed progression (Claims, Insights, Links).
    if let Some(progress) = app
        .intel_extraction_progress
        .as_ref()
        .filter(|p| !p.artifacts.is_empty())
    {
        for artifact in &progress.artifacts {
            let style = if artifact.live {
                theme::accent()
            } else {
                theme::text()
            };
            let spin = if artifact.live || intel_artifact_pending(app, &artifact.artifact) {
                format!("{} ", super::anim::spinner())
            } else {
                String::new()
            };
            // Compact step; the stall detail gets its own warning line.
            let (step, stall) = match artifact.step.split_once(" · no model activity") {
                Some((head, rest)) => (head, Some(rest.trim_start_matches(" for "))),
                None => (artifact.step.as_str(), None),
            };
            // Waiting notes ("awaiting publication", "after claims") go on
            // their own dim line so narrow panes keep the step readable.
            let (step, note) = match step.split_once(" · ") {
                Some((head, rest)) if rest.starts_with("awaiting") || rest.starts_with("after") => {
                    (head, Some(rest))
                }
                _ => (step, None),
            };
            lines.push(Line::from(Span::styled(
                format!("{spin}{} · {step}", artifact.artifact),
                if stall.is_some() {
                    theme::warn()
                } else {
                    style
                }
                .add_modifier(Modifier::BOLD),
            )));
            if let Some(note) = note {
                lines.push(Line::from(Span::styled(format!("  {note}"), theme::dim())));
            }
            if let Some(age) = stall {
                lines.push(Line::from(Span::styled(
                    format!("  ⚠ no model activity for {age}"),
                    theme::warn(),
                )));
            }
            if artifact.live && !artifact.activity.is_empty() {
                let age = progress
                    .activity_age_secs
                    .map(|s| format!(" · {s}s ago"))
                    .unwrap_or_default();
                lines.push(Line::from(Span::styled(
                    format!("  Activity: {}{age}", artifact.activity),
                    theme::dim(),
                )));
            }
        }
        if !progress.error.is_empty() && progress.state != "superseded" {
            lines.push(Line::from(Span::styled(
                format!("  {}", progress.error),
                theme::dim(),
            )));
        }
    }

    // Persisted refresh and index state, loaded outside rendering.
    let show_insights_summary = app
        .intel_extraction_progress
        .as_ref()
        .is_none_or(|p| p.artifacts.is_empty());
    let (insights_title, insights_detail) = (
        if insights_running {
            format!(
                "{} Insights · {}",
                super::anim::spinner(),
                app.intel_refresh_status
            )
        } else {
            format!("Insights · {}", app.intel_refresh_status)
        },
        None::<String>,
    );
    let insights_style = if insights_running {
        theme::accent()
    } else {
        theme::text()
    };
    if show_insights_summary {
        lines.push(Line::from(Span::styled(
            insights_title,
            insights_style.add_modifier(Modifier::BOLD),
        )));
    }
    if let Some(detail) = insights_detail {
        lines.push(Line::from(Span::styled(
            format!("  {detail}"),
            theme::dim(),
        )));
    }

    // Classifier-recommended recon mode.
}

/// Shared §6 stage rows: one immutable view for this panel, the section
/// headers, global Jobs and Logs. Only the active row spins.
fn brief_stage_lines(app: &App, lines: &mut Vec<Line<'static>>) {
    let stages = &app.intel_brief_stages;
    // Summarise from the real dependency rows: independent roots run
    // together, so there is no single "current step".
    let current = if stages.iter().all(|s| s.state == "completed") {
        format!("Brief · Complete · {0}/{0} completed", stages.len())
    } else {
        format!(
            "Brief · {}",
            intel_recon::brief_pipeline::chain_summary(stages)
        )
    };
    lines.push(Line::from(Span::styled(
        current,
        theme::text().add_modifier(Modifier::BOLD),
    )));
    for (index, stage) in stages.iter().enumerate() {
        if index > 0 {
            lines.push(Line::raw(""));
        }
        let spin = if stage.active {
            format!("{} ", super::anim::spinner())
        } else {
            String::new()
        };
        let style = if stage.active {
            theme::accent()
        } else if stage.state == "failed" {
            theme::warn()
        } else {
            theme::text()
        };
        let (head, note) = match stage.row.split_once(" · ") {
            Some((head, rest))
                if head == "Queued" || head == "Failed" || head == "Completed with warnings" =>
            {
                (head.to_string(), Some(rest.to_string()))
            }
            _ => (stage.row.clone(), None),
        };
        lines.push(Line::from(Span::styled(
            format!("{spin}{} · {head}", stage.label),
            style.add_modifier(Modifier::BOLD),
        )));
        if let Some(note) = note {
            lines.push(Line::from(Span::styled(format!("  {note}"), theme::dim())));
        }
    }
    if !app.intel_brief_publication.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("Publication / Index · {}", app.intel_brief_publication),
            theme::dim(),
        )));
    }
}

/// Markdown for a completed brief stage, from its committed output.
fn brief_stage_markdown(app: &App, op: &str) -> String {
    let Some(out) = app.intel_brief_outputs.get(op) else {
        return "_Completed_".into();
    };
    let str_of = |v: &serde_json::Value, k: &str| v[k].as_str().unwrap_or("").trim().to_string();
    let empty = || format!("_Completed · {}_", str_of(out, "empty_reason"));
    let withheld = out["withheld"]
        .as_u64()
        .filter(|n| *n > 0)
        .map(|n| format!("\n\n_{n} withheld · not supported by cited text_"))
        .unwrap_or_default();
    match op {
        "intel_brief_claims" => {
            let items = out["claims"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                return empty() + &withheld;
            }
            items
                .iter()
                .map(|c| {
                    let who = str_of(c, "attributed_to");
                    let modality = str_of(c, "modality");
                    let head = match (who.is_empty(), modality.is_empty()) {
                        (false, false) => format!("**{who}** · {modality}: "),
                        (false, true) => format!("**{who}**: "),
                        (true, false) => format!("_{modality}_: "),
                        _ => String::new(),
                    };
                    format!("- {head}{}", str_of(c, "text"))
                })
                .collect::<Vec<_>>()
                .join("\n")
                + &withheld
        }
        "intel_brief_inferences" => {
            let items = out["inferences"].as_array().cloned().unwrap_or_default();
            if items.is_empty() {
                return empty() + &withheld;
            }
            items
                .iter()
                .map(|i| {
                    let limits: Vec<String> = i["limitations"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|l| l.as_str().map(str::to_string))
                        .filter(|l| !l.is_empty())
                        .collect();
                    let mut text = format!(
                        "- **{}** · {}\n  Why: {}",
                        str_of(i, "support"),
                        str_of(i, "text"),
                        str_of(i, "why")
                    );
                    if !limits.is_empty() {
                        text.push_str(&format!("\n  Limits: {}", limits.join("; ")));
                    }
                    text
                })
                .collect::<Vec<_>>()
                .join("\n")
                + &withheld
        }
        _ => {
            let leads = out["leads"].as_array().cloned().unwrap_or_default();
            if leads.is_empty() {
                return empty();
            }
            let inferences = app
                .intel_brief_outputs
                .get("intel_brief_inferences")
                .and_then(|v| v["inferences"].as_array().cloned())
                .unwrap_or_default();
            let mut md = String::from(
                "**Potential corroboration** · metadata leads, not confirmed support\n",
            );
            for lead in leads {
                let id = str_of(&lead, "article_id");
                let title = app.intel_brief_lead_titles.get(&id).cloned().unwrap_or(id);
                let inference = inferences
                    .iter()
                    .find(|i| i["id"] == lead["inference_id"])
                    .map(|i| str_of(i, "text"))
                    .unwrap_or_default();
                md.push_str(&format!(
                    "\n- **{title}** ↔ {inference}\n  {}",
                    str_of(&lead, "why")
                ));
                let verify = str_of(&lead, "verification_needed");
                if !verify.is_empty() {
                    md.push_str(&format!("\n  Verify: {verify}"));
                }
            }
            md
        }
    }
}

fn intel_confidence_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let eval = intel_source_evaluation(app);
    use argos_osint_core::osint::source_eval::{InformationCredibility, SourceReliability};
    let source = SourceReliability::parse(&eval.reliability_letter).unwrap_or(SourceReliability::F);
    let estimate = app.intel_evidence_confidence.as_ref();
    let info = InformationCredibility::from_u8(estimate.map_or(6, |e| e.information_grade))
        .unwrap_or(InformationCredibility::CannotBeJudged);
    let (initial, provisional) = intel_recon::initial_rating(source, info);
    let score = if estimate.is_none()
        && matches!(app.intel_insight_lifecycle.as_str(), "pending" | "previous")
    {
        None
    } else {
        estimate.map_or(initial, |e| e.score)
    };
    let status = if matches!(app.intel_insight_lifecycle.as_str(), "pending" | "previous")
        && estimate.is_some()
    {
        "previous/pending"
    } else if estimate.is_none() && app.intel_insight_lifecycle == "previous" {
        "failed"
    } else if estimate.is_none() && app.intel_insight_lifecycle == "pending" {
        "pending"
    } else if estimate.map_or(provisional, |e| e.provisional) {
        "provisional"
    } else {
        "assessed"
    };
    let count = estimate
        .map(|e| format!(" · {}C+{}I", e.faithful, e.inferences))
        .unwrap_or_default();
    let lines = vec![
        Line::from(Span::styled(
            format!(
                "Source {} · {}",
                source.as_str(),
                sentence_label(source.label())
            ),
            theme::text(),
        )),
        Line::from(Span::styled(
            format!(
                "Information {} · {}",
                info.as_u8(),
                sentence_label(info.label())
            ),
            theme::text(),
        )),
        Line::from(Span::styled(
            format!(
                "Brief rating {} · {status}{count}",
                score
                    .map(|s| format!("{s}/100"))
                    .unwrap_or_else(|| "N/A".into())
            ),
            theme::accent().add_modifier(Modifier::BOLD),
        )),
    ];
    let digit = info.as_u8().to_string();
    let art = grade_ascii_lines(&format!("{}{digit}", eval.reliability_letter));
    let art_width = art.iter().map(|row| row.chars().count()).max().unwrap_or(0);
    let style = Style::default()
        .fg(grade_score_color(source.meter()))
        .add_modifier(Modifier::BOLD);
    if width >= art_width + 16 {
        let mut details = Vec::new();
        for line in lines {
            let text = line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>();
            let row_style = line
                .spans
                .first()
                .map(|span| span.style)
                .unwrap_or_default();
            details.extend(
                wrap_text(&text, width - art_width - 2)
                    .into_iter()
                    .map(|row| Line::from(Span::styled(row, row_style))),
            );
        }
        (0..art.len().max(details.len()))
            .map(|i| {
                let badge = art.get(i).cloned().unwrap_or_else(|| " ".repeat(art_width));
                let padding = art_width.saturating_sub(badge.chars().count()) + 2;
                let mut spans = vec![Span::styled(badge, style), Span::raw(" ".repeat(padding))];
                if let Some(detail) = details.get(i) {
                    spans.extend(detail.spans.clone());
                }
                Line::from(spans)
            })
            .collect()
    } else {
        let mut badge: Vec<_> = art
            .into_iter()
            .map(|row| Line::from(Span::styled(row, style)))
            .collect();
        badge.extend(lines);
        badge
    }
}

fn sentence_label(label: &str) -> String {
    let mut chars = label.to_lowercase().chars().collect::<Vec<_>>();
    if let Some(first) = chars.first_mut() {
        first.make_ascii_uppercase();
    }
    chars.into_iter().collect()
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
        .map(|row| glyphs.iter().map(|g| g[row]).collect::<Vec<_>>().join(" "))
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
        '1' => [" ██╗", "███║", "╚██║", " ██║", " ██║", " ╚═╝"],
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
}

fn intel_source_evaluation(app: &App) -> IntelSourceEval {
    use argos_osint_core::osint::SourceReliability;
    let reliability = app
        .intel_publisher_prior
        .as_ref()
        .and_then(|p| SourceReliability::parse(&p.observation.reliability))
        .unwrap_or(SourceReliability::F);
    IntelSourceEval {
        reliability_letter: reliability.as_str().into(),
    }
}

fn draw_intel_tags(
    frame: &mut Frame,
    article: &argos_osint_core::store::AtlasArticleRow,
    area: Rect,
) {
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum IntelBodyPresentation<'a> {
    Content {
        markdown: &'a str,
        warning: Option<String>,
    },
    Retrieving,
    Status(String),
}

pub(crate) fn intel_body_presentation(app: &App) -> IntelBodyPresentation<'_> {
    let Some(article) = app.intel_articles.get(app.intel_sel) else {
        return IntelBodyPresentation::Status("No article selected.".into());
    };
    if app
        .intel_body_owner
        .as_ref()
        .is_some_and(|(id, job)| id != &article.id || job != &app.intel_brief_job)
    {
        return IntelBodyPresentation::Status(
            "Article snapshot pending · use Reload to retry.".into(),
        );
    }
    if let Some(markdown) = app
        .intel_brief_outputs
        .get("intel_brief_body")
        .and_then(|v| v["markdown"].as_str())
        .filter(|s| !s.trim().is_empty())
    {
        return IntelBodyPresentation::Content {
            markdown,
            warning: None,
        };
    }
    let retrieving = app.intel_brief_job.is_some()
        && app.intel_brief_stages.iter().any(|stage| {
            stage.operation == "intel_brief_body" && stage.active && stage.state == "running"
        });
    if let Some(body) = app.intel_body.as_ref().filter(|body| {
        !app.intel_body_require_commit
            && (body.state != "running" || retrieving)
            && body.article_id == article.id
            && matches!(body.quality.as_str(), "complete" | "partial" | "uncertain")
            && !body.body_markdown.trim().is_empty()
    }) {
        return IntelBodyPresentation::Content {
            markdown: &body.body_markdown,
            warning: (body.quality != "complete")
                .then(|| format!("[{}] {}", body.quality, body.quality_rationale)),
        };
    }
    if let Some(error) = &app.intel_body_read_error {
        return IntelBodyPresentation::Status(format!(
            "Could not read full article: {error} · use Reload to retry."
        ));
    }
    // Only the current job's acquisition/cleanup stages establish activity.
    if app.intel_brief_job.is_some() {
        if retrieving {
            return IntelBodyPresentation::Retrieving;
        }
        if let Some(stage) = app.intel_brief_stages.iter().find(|stage| {
            matches!(stage.operation.as_str(), "intel_brief_body") && stage.state != "completed"
        }) {
            return IntelBodyPresentation::Status(format!(
                "{} · {} · use Reload to retry.",
                stage.label, stage.row
            ));
        }
    }
    if app
        .intel_body
        .as_ref()
        .is_some_and(|body| body.state == "running")
    {
        return IntelBodyPresentation::Status(
            "Retrieval is no longer active · use Reload to retry.".into(),
        );
    }
    if app
        .intel_brief_publication
        .starts_with("Brief job unavailable")
    {
        return IntelBodyPresentation::Status(app.intel_brief_publication.clone());
    }
    let reason = app
        .intel_body
        .as_ref()
        .map(|body| body.quality_rationale.as_str())
        .filter(|reason| !reason.trim().is_empty())
        .unwrap_or("No current full article available");
    IntelBodyPresentation::Status(format!(
        "{reason} · use Reload to retrieve the full article."
    ))
}

pub(crate) fn intel_body_has_content(app: &App) -> bool {
    matches!(
        intel_body_presentation(app),
        IntelBodyPresentation::Content { .. }
    )
}

fn intel_brief_full_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    match intel_body_presentation(app) {
        IntelBodyPresentation::Retrieving => Vec::new(),
        IntelBodyPresentation::Status(reason) => super::components::measured_lines(
            vec![Line::styled(reason, theme::dim())],
            width.max(1).min(u16::MAX as usize) as u16,
        ),
        IntelBodyPresentation::Content { markdown, warning } => {
            let mut lines = Vec::new();
            if let Some(warning) = warning {
                lines.push(Line::styled(warning, theme::dim()));
            }
            if app.intel_full_collapsed {
                lines.push(Line::styled(
                    "[collapsed — press Reload to retrieve again]",
                    theme::dim(),
                ));
            } else {
                lines.extend(
                    super::markdown::markdown_lines(markdown, width.max(1))
                        .into_iter()
                        .map(md_line_to_line),
                );
            }
            lines
        }
    }
}

pub(crate) fn intel_body_loading(app: &App) -> bool {
    matches!(
        intel_body_presentation(app),
        IntelBodyPresentation::Retrieving
    )
}

/// True while cleaned-body insight re-extract is resolving extracted/confidence values.
pub(crate) fn intel_insights_loading(app: &App) -> bool {
    let Some(article) = app.intel_articles.get(app.intel_sel) else {
        return false;
    };
    app.intel_insights_running.contains(&article.id)
}

fn draw_intel_body_loading(frame: &mut Frame, viewport: Rect, area: AbsRect, app: &App) {
    let Some((vis, _)) = intersect_abs(viewport, area) else {
        return;
    };
    frame.render_widget(Block::default().style(theme::text()), vis);
    stroke_clipped_box(
        frame,
        viewport,
        area,
        if app.focus == Target::IntelFullArticle {
            " full article • focused "
        } else {
            " full article "
        },
    );

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
    draw_centered_loading_card(frame, inner_vis, &label, detail.as_deref());
}

/// Centered loading card over a briefing side pane (extracted / confidence).
fn intel_insights_progress_detail(app: &App) -> Option<String> {
    let message = app.intel_body_message.trim();
    if message.is_empty() {
        return None;
    }
    let lower = message.to_ascii_lowercase();
    if lower.contains("re-extract") || lower.contains("insight") {
        Some(message.to_string())
    } else {
        None
    }
}

/// Popup card: bordered block horizontally and vertically centered in `area`.
fn draw_centered_loading_card(frame: &mut Frame, area: Rect, label: &str, detail: Option<&str>) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let mut lines = vec![Line::from(Span::styled(label.to_string(), theme::dim()))];
    if let Some(detail) = detail.filter(|text| !text.is_empty()) {
        lines.push(Line::from(Span::styled(detail.to_string(), theme::dim())));
    }
    let content_h = lines.len() as u16;
    let text_w = lines
        .iter()
        .map(|line| line.width() as u16)
        .max()
        .unwrap_or(0)
        .max(16);
    let desired_w = text_w.saturating_add(4).max(18);
    let card_w = desired_w.min(area.width).max(1);
    let desired_h = content_h.saturating_add(2).max(3);
    let card_h = desired_h.min(area.height).max(1);
    let x = area.x + area.width.saturating_sub(card_w) / 2;
    let y = area.y + area.height.saturating_sub(card_h) / 2;
    let card = Rect {
        x,
        y,
        width: card_w,
        height: card_h,
    };
    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme::BORDER).bg(theme::BG))
                .style(theme::text()),
        ),
        card,
    );
}

/// Mirror `insights_progress_lines`: braille spinner + phase label, optional detail.
fn intel_body_progress_lines(app: &App) -> (String, Option<String>) {
    let message = app
        .intel_brief_stages
        .iter()
        .find(|stage| {
            stage.operation == "intel_brief_body" && stage.active && stage.state == "running"
        })
        .map(|stage| stage.row.trim_start_matches("Running · "))
        .unwrap_or("");
    let lower = message.to_ascii_lowercase();
    let phase = if lower.contains("clean") {
        "Cleaning full article"
    } else if lower.contains("re-extract") || lower.contains("insight") {
        // Body is already painted; insight phase is shown on extracted/confidence cards.
        "Full article ready"
    } else if lower.contains("classif") || lower.contains("sponsored") {
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
    let label = format!("{} {phase}", super::anim::spinner());
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
        let (left, center, right) = intel_briefing_areas(app, body);
        if contains(left, x, y) {
            return Some(Target::IntelLeftColumn);
        }
        if let Some(layout) = intel_center_layout(app, center) {
            let (visit, reload) = intel_reader_actions(layout.reload);
            if abs_contains(visit, x, y) && contains(center, x, y) {
                return Some(Target::Button(ButtonId::IntelVisit));
            }
            if abs_contains(reload, x, y) && contains(center, x, y) {
                return Some(Target::Button(ButtonId::IntelBodyRefresh));
            }
        }
        let rows = intel_briefing_right_rows(app, right);
        if contains(rows[2], x, y) {
            // Clicking the jobs pane focuses job controls.
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
        let index = app.scrolls.intel_list as usize
            + super::components::ListGeometry::fixed_with_gap(app.intel_articles.len(), 1, 0)
                .index_at((y - list.y - 1) as usize)?;
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

fn draw_atlas_coverage(frame: &mut Frame, app: &App, area: Rect) {
    let coverage = &app.atlas_stats.coverage;
    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                "Classifications {}/{}",
                coverage.classified, coverage.articles
            ),
            theme::text(),
        )),
        Line::from(Span::styled(
            format!(
                "Unresolved: {} failed · {} blocked",
                coverage.classification_failed, coverage.classification_blocked
            ),
            theme::dim(),
        )),
        Line::from(format!(
            "Entities {} available · {} unavailable",
            coverage.entities_available, coverage.entities_unavailable
        )),
        Line::from(format!(
            "Publisher priors {} available · {} unavailable",
            coverage.publishers_available, coverage.publishers_unavailable
        )),
        Line::from(vec![
            Span::raw(format!(
                "Related {} pairs · shared mentions only",
                coverage.related_pairs
            )),
            Span::styled(format!(" · Warnings {}", coverage.warnings), theme::dim()),
        ]),
        Line::from(Span::styled(
            if atlas_extracting(app) {
                format!("{} Stage: {}", super::anim::spinner(), app.atlas_status)
            } else {
                format!("Stage: {}", app.atlas_status)
            },
            theme::accent(),
        )),
    ];
    if coverage.legacy_unknown > 0 {
        lines.insert(
            1,
            Line::from(Span::styled(
                format!(
                    "Legacy unk provenance: {} · not reclassified",
                    coverage.legacy_unknown
                ),
                theme::dim(),
            )),
        );
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(pane(" coverage "))
            .wrap(Wrap { trim: false })
            .scroll((app.scrolls.insights, 0)),
        area,
    );
}

/// Native metadata and classifier work use the live stage label.
pub(crate) fn atlas_extracting(app: &App) -> bool {
    app.atlas_pause.is_some() && !app.atlas_status.is_empty()
}

fn draw_atlas_live(frame: &mut Frame, app: &App, area: Rect) {
    let (actions, table, insights, feed) = atlas_live_areas(app, area);
    if area.width < 100 || area.height < 30 {
        pane_tabs(
            frame,
            app,
            Rect::new(area.x, area.y + ACTION_H, area.width, 1),
            &["Origins", "Coverage", "Headlines"],
        );
    }
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
    app.layout
        .borrow_mut()
        .register(Target::AtlasCycleStats, table);
    let origins: Vec<&atlas::OriginStat> = app
        .atlas_stats
        .origins
        .iter()
        .filter(|row| row.articles > 0 || !app.atlas_stats.scored)
        .collect();
    super::atlas_table::draw_origins(
        frame,
        super::atlas_table::OriginsView {
            area: table,
            origins: &origins,
            stats: &app.atlas_stats,
            scroll: app.scrolls.origins as usize,
            title: " origins ",
            empty: "No country statistics yet.",
            prefix: Vec::new(),
            focused: app.focus == Target::AtlasCycleStats,
        },
    );
    draw_atlas_coverage(frame, app, insights);
    let rows = inset(feed);
    for row in 0..super::components::ListGeometry::capacity_with_gap(rows.height as usize, 1, 0) {
        let index = app.scrolls.atlas_feed as usize + row;
        if index < app.atlas_feed.len() {
            app.layout.borrow_mut().register(
                Target::AtlasFeed(index),
                Rect::new(rows.x, rows.y + row as u16, rows.width, 1),
            );
        }
    }
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
    let rows = inset(cycles);
    for row in 0..super::components::ListGeometry::capacity_with_gap(rows.height as usize, 1, 0) {
        let index = app.scrolls.atlas_runs as usize + row;
        if index < app.atlas_runs.len() {
            app.layout.borrow_mut().register(
                Target::AtlasHistory(index),
                Rect::new(rows.x, rows.y + row as u16, rows.width, 1),
            );
        }
    }
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
            .map(|(index, run)| news_cycle_line(run, width, index == app.atlas_run_sel))
            .collect()
    };
    frame.render_widget(Paragraph::new(lines).block(pane(" news cycle ")), cycles);
    let buttons = button_areas(actions, 3);
    let live = atlas_history_live_label(app);
    draw_button(frame, app, ButtonId::AtlasLive, &live, buttons[0]);
    if app.atlas_can_resume() {
        draw_button(frame, app, ButtonId::AtlasResume, "Resume", buttons[1]);
    } else {
        // Dimmed: the selected cycle has nothing to resume (Enter explains why).
        draw_button_state(
            frame,
            app,
            ButtonId::AtlasResume,
            "Resume",
            buttons[1],
            false,
        );
    }
    draw_button(frame, app, ButtonId::AtlasDelete, "Delete", buttons[2]);
}

/// Date on the left, pipeline state on the right.
fn news_cycle_line(
    run: &argos_osint_core::store::AtlasRunRow,
    width: usize,
    selected: bool,
) -> Line<'static> {
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
        return Line::from(Span::styled(fit(&format!("{when} {status}"), width), style));
    }
    Line::from(vec![
        Span::styled(when, style),
        Span::styled(" ".repeat(gap), style),
        Span::styled(status.to_string(), style),
    ])
}

fn draw_atlas_cycle_stats(frame: &mut Frame, app: &App, area: Rect) {
    let stats = selected_cycle_stats(app);
    let origins: Vec<&atlas::OriginStat> = stats
        .origins
        .iter()
        .filter(|row| row.articles > 0)
        .collect();
    let summary = format!(
        "Articles {} · classified {} · Related {} pairs",
        stats.article_total(),
        stats.coverage.classified,
        stats.coverage.related_pairs
    );
    let coverage = if argos_osint_core::atlas_work::is_collection_contract(&stats.contract_version)
    {
        let mut line = format!(
            "Entities {} · publisher priors {} · warnings {}",
            stats.coverage.entities_available,
            stats.coverage.publishers_available,
            stats.coverage.warnings
        );
        // Phase 6 (v2): Brain actor indexing coverage.
        if stats.contract_version == argos_osint_core::atlas_work::COLLECTION_CONTRACT {
            line.push_str(&format!(
                " · actors {}/{}{}",
                stats.coverage.actor_indexed,
                stats.coverage.articles,
                if stats.coverage.actor_index_failed > 0 {
                    format!(" · {} failed", stats.coverage.actor_index_failed)
                } else {
                    String::new()
                }
            ));
        }
        line
    } else {
        format!("Legacy memory publication · {}", stats.memories.line())
    };
    app.layout
        .borrow_mut()
        .register(Target::AtlasCycleStats, area);
    super::atlas_table::draw_origins(
        frame,
        super::atlas_table::OriginsView {
            area,
            origins: &origins,
            stats: &stats,
            scroll: app.scrolls.origins as usize,
            title: " stats ",
            empty: "No country statistics for this cycle.",
            prefix: vec![
                Line::from(Span::styled(summary, theme::dim())),
                Line::from(Span::styled(coverage, theme::dim())),
            ],
            focused: app.focus == Target::AtlasCycleStats,
        },
    );
}

fn draw_map_or_hold(frame: &mut Frame, app: &App, area: Rect) {
    if app.atlas_map_hold {
        frame.render_widget(Clear, area);
        let block = pane(" map ");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        super::anim::draw_centered(frame.buffer_mut(), inner, "Loading country", theme::dim());
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
    let rows = inset(list);
    for row in 0..super::components::ListGeometry::capacity_with_gap(rows.height as usize, 1, 0) {
        let index = app.scrolls.atlas_news as usize + row;
        if index < app.atlas_articles.len() {
            app.layout.borrow_mut().register(
                Target::AtlasArticle(index),
                Rect::new(rows.x, rows.y + row as u16, rows.width, 1),
            );
        }
    }
    let width = inset(list).width as usize;
    let lines = if app.atlas_articles.is_empty() {
        vec![Line::from(Span::styled(
            if app.atlas_news_loading() {
                super::anim::loading("Loading cycle articles")
            } else {
                "No saved articles for this run.".into()
            },
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
    let row = (y - inner.y) as usize;
    Some(scroll as usize + row)
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
            let slots = button_areas(actions, 3);
            let buttons = [
                ButtonId::AtlasLive,
                ButtonId::AtlasResume,
                ButtonId::AtlasDelete,
            ];
            return slots
                .iter()
                .position(|slot| contains(*slot, x, y))
                .map(|index| Target::Button(buttons[index]));
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
    let (actions, table, _insights, feed) = atlas_live_areas(app, body);
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
    if contains(table, x, y) {
        return Some(Target::AtlasCycleStats);
    }
    if contains(feed, x, y) {
        let index = atlas_row_at(feed, app.scrolls.atlas_feed, y)?;
        return (index < app.atlas_feed.len()).then_some(Target::AtlasFeed(index));
    }
    None
}

/// Host lines for System (existing hardware profile fields only).
pub(crate) fn system_host_lines(hw: &argos_osint_core::hardware::HardwareProfile) -> Vec<String> {
    let mut lines = vec![hw.one_line()];
    lines.push(format!("OS: {} · {}", hw.os, hw.arch));
    if !hw.cpu_name.is_empty() {
        lines.push(format!("CPU: {}", hw.cpu_name));
    }
    lines.push(format!(
        "Cores: {} logical{}",
        hw.logical_cores,
        hw.physical_cores
            .map(|n| format!(" · {n} physical"))
            .unwrap_or_default()
    ));
    lines.push(format!(
        "Memory: {:.1} GB total · {:.1} GB available",
        hw.total_ram_gb, hw.available_ram_gb
    ));
    lines.push(format!("Backend: {}", hw.backend));
    if let Some(err) = &hw.gpu_error {
        lines.push(format!("GPU probe: {err}"));
    }
    if hw.disk_total_gb > 0.0 {
        lines.push(format!(
            "Disk: {:.0} GB total · {:.0} GB free",
            hw.disk_total_gb, hw.disk_available_gb
        ));
    }
    lines
}

/// Paths for System: configuration and database always; other data, index,
/// and cache paths only when they exist.
pub(crate) fn system_path_lines() -> Vec<String> {
    use argos_osint_core::paths;
    let mut lines = vec![
        format!("Config: {}", paths::config_path().display()),
        format!("Database: {}", paths::db_path().display()),
    ];
    let optional = [
        ("Data", paths::home_dir()),
        ("Memory index", paths::lancedb_dir()),
        ("Credentials", paths::auth_path()),
        ("Hardware cache", paths::hardware_cache_path()),
    ];
    for (label, path) in optional {
        if path.exists() {
            lines.push(format!("{label}: {}", path.display()));
        }
    }
    lines
}

/// Profile: the Overview/System tabs plus the Configs popup.
fn draw_system(frame: &mut Frame, app: &App, area: Rect) {
    // The Profile module owns its whole body, including the tab strip: the two
    // tabs split the old System pane rather than adding a pane beside it.
    super::profile::draw_profile(frame, app, area);
}

fn popup_text(app: &App) -> String {
    match &app.overlay {
        Overlay::Configs => String::new(),
        Overlay::Help => {
            let mut lines = vec![
                format!("# {} commands", app.module.map_or("Home", ModuleId::title)),
                String::new(),
            ];
            for command in super::commands::matching("", app.module) {
                if command.module.is_some() && command.module != app.module {
                    continue;
                }
                if command.category == "Navigation" && command.id != "home" {
                    continue;
                }
                let shortcut = if command.shortcut.is_empty() {
                    String::new()
                } else {
                    format!(" (`{}`)", command.shortcut)
                };
                lines.push(format!(
                    "- **{}**{} — {}",
                    command.label, shortcut, command.description
                ));
            }
            lines.push(String::new());
            lines.push(help_text(app).replace('\n', "  \n"));
            lines.join("\n")
        }
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

        Overlay::Palette => app
            .palette_items()
            .into_iter()
            .map(|item| item.label)
            .collect::<Vec<_>>()
            .join("\n"),
        Overlay::AddFallback => String::new(),
        Overlay::ResumeSession(_) => "Open last app view session".to_string(),
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
        None => "Home\n\n↑↓ or j/k select an application\nEnter opens it\n1 Intel · 2 Atlas · 3 Brain · 4 Recon · 5 Jobs · 6 Logs · 7 Tools · 8 Models · 9 Profile\nNumber keys switch apps when you are not typing in a field\nCtrl+K command palette · ? help · Esc closes this card\nCtrl+C quits when nothing is running · Ctrl+Q quits from anywhere",
        Some(ModuleId::Intel) => "Intel\n\nBulletin board browses Atlas-stored headlines by classification\nSix tabs: Geopolitical Economic Military Information Stability Tech\nThe day button filters by Atlas news-cycle run day\nSearch filters title, description, source, and URL\n↑↓ select a story · the hero updates with the selection\nEnter opens Briefing Focus for that article\nBriefing shows preview, full article, extracted claims/inferences/context/links, and confidence\nClick a claim or actor, or focus it and press Enter, to map its two-hop neighborhood\nTopology selection inspects; Recenter changes the seed; Back restores this brief\nRelated beneath Full Article reflects ingest associations with at least two shared entities\nJobs pane tracks focus-brief background progress\nEsc returns from briefing to bulletin, or from bulletin to home",
        Some(ModuleId::Atlas) => "Atlas\n\nNews cycle is the view that opens. Go Live shows the pipeline\nRun starts the pipeline. Pause parks it after the current request\nResume continues that run. Ctrl+C pauses\nAuto Run starts the pipeline now and again every 60 minutes until it is turned off\nThe button shows when the next run starts. A manual run moves that time out by 60 minutes\nThe table shows country heat. The feed lists headlines from this session\n↑↓ move through headlines · the wheel and Ctrl+U/D scroll that list\nEnter or click opens the selected headline\nFailed requests, including rate limits, are written to Logs\nEnter on a ▸ error there opens the full API response\nNews cycle lists saved cycles by date and status. Enter or click opens that cycle's news feed\nStats for the selected cycle sit under the map, left of the list\nClick the stats pane, then ↑↓ or the wheel scrolls the country table\nThe world map sits above those panes and takes most of the view\nGo Live, Resume, and Delete sit between the map and those panes. Resume continues retained collection/classification and native metadata. Coverage shows classifications, entities, publisher priors, and Related pairs. Atlas collects articles without creating Brain claim memories. When auto run is on, Go Live counts down\nThe map follows the selected news cycle. It does not take keys or clicks\nTier 1 and 2 countries are named in full. Tier 3 shows the country code\nAnother news cycle row recolours the map and replaces the stats\nThe news list shows the title, then publisher, country code, and category\nEnter or click opens the article and zooms the map to its country\nWorld map restores the news cycle list and zooms back out\nDelete removes the selected cycle. Backspace does the same when a cycle is focused\nEsc on the news feed or on Live returns to news cycle\nEsc on news cycle returns home",
        Some(ModuleId::Recon) if !app.recon_chat => "Recon investigations\n\nThe list is the most recent investigations\n↑↓ move · Enter opens the transcript\nNew starts an investigation · Delete removes the selected one\nType to search titles\nEsc returns home · Ctrl+N new investigation",
        Some(ModuleId::Recon) => "Recon investigation\n\nEnter sends · Shift+Enter inserts a line · / opens commands\nTab moves between transcript and prompt\n↑↓ select a query, plan, evidence activity, or answer\n←→ or h/l fold the selected Plan or activity\nEnter toggles that fold · o inspects the captured source · f opens full text\n◉ brain opens memories used by Synthesis\nrecall: off skips insight extraction. recall: on writes claims for later answers\nCtrl+K command palette · Ctrl+U/Ctrl+D scroll\nEsc returns to investigations · Ctrl+C cancels a running turn\nCtrl+N new thread · Alt+←/→ recent threads",
        Some(ModuleId::System) => "Profile\n\nTab / Shift+Tab traverse controls; Enter activates\nOverview / System / Configs · t switches analytics/host · x Configs\n0 Summary · 1–5 Intel/Recon/Atlas/Models/Tools · [ / ] apps\nTab reveals panels · Arrows select rows/buckets · Enter or m expands\nLeft/Right selects buckets · v chart/table · s sort · g provider fold · PageUp/PageDown or wheel scroll\np period · f six dimension filters · c clears filters\nPicker: type to search, Tab dimension, arrows select, Enter apply\nr refreshes statistics in Overview, hardware in System\nEsc restores grid or returns home · Ctrl+K commands",
        Some(ModuleId::Logs) => "Logs\n\nDurable events from every app and background worker, kept 24 hours\nThe header counts errors, warnings, and failures in the last hour\nFilter narrows by text. Level, App, and the job filter narrow further\n↑↓ select an event · Enter or click folds its detail\nf toggles live follow. Moving off the newest event pauses it\no or Open job shows the event's job in Jobs\nOpened from Jobs, Esc or Back to job returns there\nClear events removes events only; jobs, results, and memories stay\nCtrl+U/Ctrl+D and the wheel scroll the list",
        Some(ModuleId::Jobs) => "Jobs\n\nBackground work with timing, attempts, and errors\nActive work is listed first, then recent history, then service workers\nStatus and App filter the table. Filter matches title, id, operation, or error\n↑↓ select a job · Enter opens its detail (full screen when narrow)\nl or View logs opens Logs filtered to the job and its phases\nRetry failed requeues only failed index or summary tasks; completed work is kept\nOpen source jumps to the Atlas cycle or investigation when there is one\nClear completed / stopped / failed removes that history globally; results stay\nc or Stop job stops the selected supported job at a safe point\nService workers stay available while idle; elapsed is their age, not one request\nOld jobs are removed automatically after 72 hours\nUnknown historic timing shows Unavailable\nEsc closes the detail, then returns home",
        Some(ModuleId::Brain) => "Brain\n\nKind cycles All → Claims → Actors. All contains Claims and unique Actor-name groups\nFind filters after actor names are grouped; grouping does not merge identities\nCreate saves a user-supplied, unassessed Claim. Pin/Delete apply to Claims\nEnter opens Claim detail or an Actor group\nClaims show a graph above Related and an independent Summary\nActor groups show Graph and Related side by side, or reachable compact pages\nClick the Claim path or Actor group graph, or focus it and press Enter, to map that exact record\nTopology selection inspects; Recenter changes the seed; Back restores the originating detail\nActor Related tabs: Claims / Threads / Articles. Claim tabs: Actors / Sources\nEach tab keeps its own selection and scroll. See more loads another page\nEnter or click a Related row opens its Claim, Actor group, Recon thread or exact Intel article\nActor Articles reflect ingest associations with at least two distinct shared entities\nClaim Sources preserve original provenance; investigation detections require Recall on\nEsc or Back restores the previous record, tab, row, offsets and focus across apps\nTab moves through reachable controls and panes; arrows, wheel and PageUp/PageDown scroll\nClaims keep their saved summary; Actor groups show local association totals\nEsc returns home from the list · ? opens this card",
        Some(ModuleId::Providers) => "Models\n\nEach account tab stores that provider only\nDefaults sets every role's primary model and ordered fallbacks\nFallbacks are tried top to bottom after primary retries (4 then 3 each)\nAdd fallback opens Google, Nvidia, or OpenRouter catalogs\nProvider and Model open the accounts and models that connection can use\n↑↓ choose · Enter selects · Esc closes the list\nEsc returns home · ? opens this card",
        _ => "Controls\n\nTab moves between fields and buttons\n1–9 switch apps when not typing in a field\nEnter activates the focused control\n↑↓ move through lists\nCtrl+U/Ctrl+D and the wheel scroll the pane under the pointer\nTyping works only in a focused field\nEsc returns home · ? opens this card",
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
    if matches!(&app.overlay, Overlay::Help)
        || matches!(&app.overlay, Overlay::Block { title, .. } if (title.to_ascii_lowercase().contains("report") || title == "Full article"))
    {
        let md = super::markdown::markdown_lines(&popup_text(app), width.max(1));
        return md.into_iter().map(md_line_to_line).collect();
    }
    popup_text(app)
        .lines()
        .map(|line| Line::from(line.to_string()))
        .collect()
}

fn add_fallback_popup_area(screen: Rect) -> Rect {
    popup_area(screen)
}

fn add_fallback_layout(area: Rect) -> Vec<Rect> {
    let inner = inset(area);
    split_vertical(
        inner,
        [
            Constraint::Length(ACTION_H),
            Constraint::Length(2),
            Constraint::Length(FIELD_H),
            Constraint::Min(4),
            Constraint::Length(ACTION_H),
        ],
    )
}

fn draw_add_fallback(frame: &mut Frame, app: &App) {
    let area = add_fallback_popup_area(frame.area());
    cover(frame, area);
    let title = format!(" Add fallback · {} ", app.defaults_role.label());
    frame.render_widget(Paragraph::new("").block(theme::card(&title)), area);
    let close = Rect {
        x: area.x + area.width.saturating_sub(8),
        y: area.y,
        width: 8.min(area.width),
        height: 1,
    };
    app.layout
        .borrow_mut()
        .register(Target::CloseOverlay, close);
    frame.render_widget(Paragraph::new(" close ").style(theme::card_accent()), close);

    let rows = add_fallback_layout(area);
    let tabs = [
        ProviderPage::Google,
        ProviderPage::Nvidia,
        ProviderPage::OpenRouter,
    ];
    let tab_rects = button_areas(rows[0], tabs.len());
    for (page, rect) in tabs.into_iter().zip(tab_rects) {
        app.layout
            .borrow_mut()
            .register(Target::Button(ButtonId::FallbackTab(page)), rect);
        let selected = app.fallback_popup_tab == page;
        draw_button_state(
            frame,
            app,
            ButtonId::FallbackTab(page),
            page.title(),
            rect,
            selected,
        );
    }

    let provider = match app.fallback_popup_tab {
        ProviderPage::Google => "google",
        ProviderPage::Nvidia => "nvidia",
        ProviderPage::OpenRouter => "openrouter",
        ProviderPage::Defaults => "openrouter",
    };
    let secret = provider::account_secret(&app.auth, provider);
    let has_key = !secret.api_key.as_deref().unwrap_or("").trim().is_empty();
    let status = if !has_key {
        format!(
            "No API key. Save one on the {} tab, then add a fallback.",
            app.fallback_popup_tab.title()
        )
    } else if app.catalog_for == provider && app.model_catalog.is_empty() {
        "This account returned no models.".into()
    } else if app.catalog_for != provider {
        super::anim::loading("Loading models this account can call")
    } else {
        format!(
            "{} models · unknown capabilities stay selectable",
            app.model_catalog.len()
        )
    };
    frame.render_widget(
        Paragraph::new(status)
            .style(theme::dim())
            .wrap(Wrap { trim: true }),
        rows[1],
    );

    draw_field(frame, app, FieldId::FallbackFilter, " Filter ", rows[2]);
    app.layout
        .borrow_mut()
        .register(Target::Field(FieldId::FallbackFilter), rows[2]);

    let models = app.filtered_fallback_models();
    let list_inner = inset(rows[3]);
    frame.render_widget(pane(" Models "), rows[3]);
    let mut lines = Vec::new();
    if !has_key {
        lines.push(Line::from(Span::styled(
            "Add is disabled until a key is saved.",
            theme::dim(),
        )));
    } else if models.is_empty() && !app.fallback_popup_filter.trim().is_empty() {
        lines.push(Line::from("No models match this filter."));
    } else if models.is_empty() {
        lines.push(Line::from(Span::styled("No models loaded.", theme::dim())));
    } else {
        let start = app.scrolls.popup as usize;
        for (i, model) in
            models
                .iter()
                .enumerate()
                .skip(start)
                .take(super::components::ListGeometry::capacity(
                    list_inner.height as usize,
                    1,
                ))
        {
            if i > start {
                lines.push(Line::raw(""));
            }
            let y = list_inner.y.saturating_add((i - start) as u16 * 2);
            if y < list_inner.y.saturating_add(list_inner.height) {
                app.layout.borrow_mut().register(
                    Target::Button(ButtonId::FallbackPick(i)),
                    Rect {
                        x: list_inner.x,
                        y,
                        width: list_inner.width,
                        height: 1,
                    },
                );
            }
            let mut label = format!("{} · {}", model.name, model.id);
            if let Some(reason) = app.fallback_incompatible(&model.id) {
                label.push_str(" · ");
                label.push_str(reason);
            }
            let style = if i == app.fallback_popup_sel {
                theme::selected()
            } else {
                theme::text()
            };
            lines.push(Line::from(Span::styled(
                super::components::clip_text(&label, list_inner.width as usize),
                style,
            )));
        }
    }
    let visible: Vec<Line> = lines;
    frame.render_widget(Paragraph::new(visible), list_inner);

    let add_enabled = has_key && !models.is_empty();
    draw_button_state(
        frame,
        app,
        ButtonId::ConfirmAddFallback,
        "Add fallback",
        rows[4],
        add_enabled && app.focus == Target::Button(ButtonId::ConfirmAddFallback),
    );
}

fn draw_resume_session(frame: &mut Frame, app: &App, session: &LastViewSession) {
    let area = active_popup_area(app, frame.area());
    cover(frame, area);
    frame.render_widget(
        Paragraph::new("").block(theme::card(" Open last app view session ")),
        area,
    );

    let inner = inset(area);
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "A previous session was detected:",
        theme::text().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));

    let mod_title = session.module.as_deref().unwrap_or("Unknown");
    lines.push(Line::from(vec![
        Span::styled("App  ", theme::dim()),
        Span::styled(
            mod_title.to_ascii_uppercase(),
            theme::accent().add_modifier(Modifier::BOLD),
        ),
    ]));

    if let Some(page) = &session.intel_page {
        let art = session.intel_article_title.as_deref().unwrap_or("");
        let detail = if art.is_empty() {
            page.clone()
        } else {
            format!("{page} · {art}")
        };
        lines.push(Line::from(vec![
            Span::styled("Screen  ", theme::dim()),
            Span::styled(fit(&detail, inner.width as usize - 12), theme::text()),
        ]));
    } else if let Some(tid) = &session.recon_thread_id {
        let t_title = session
            .recon_thread_title
            .as_deref()
            .unwrap_or(tid.as_str());
        lines.push(Line::from(vec![
            Span::styled("Screen  ", theme::dim()),
            Span::styled(fit(t_title, inner.width as usize - 12), theme::text()),
        ]));
    } else if let Some(mem) = &session.memory_title {
        lines.push(Line::from(vec![
            Span::styled("Screen  ", theme::dim()),
            Span::styled(fit(mem, inner.width as usize - 12), theme::text()),
        ]));
    } else if let Some(page) = &session.atlas_page {
        lines.push(Line::from(vec![
            Span::styled("Screen  ", theme::dim()),
            Span::styled(page, theme::text()),
        ]));
    } else if let Some(tool) = &session.osint_tool_id {
        lines.push(Line::from(vec![
            Span::styled("Tool  ", theme::dim()),
            Span::styled(tool, theme::text()),
        ]));
    } else if let Some(page) = &session.providers_page {
        lines.push(Line::from(vec![
            Span::styled("Page  ", theme::dim()),
            Span::styled(page, theme::text()),
        ]));
    }

    let content_height = inner.height.saturating_sub(2);
    let text_h = (lines.len() as u16).min(content_height);
    let text_y = inner.y + content_height.saturating_sub(text_h) / 2;
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .style(theme::card_text()),
        Rect {
            x: inner.x,
            y: text_y,
            width: inner.width,
            height: text_h,
        },
    );

    let btn_y = inner.y + inner.height.saturating_sub(1);
    let gap = 3u16;
    let confirm_w = 20.min(inner.width.saturating_sub(gap) / 2);
    let dismiss_w = 23.min(inner.width.saturating_sub(gap) / 2);
    let btn_x = inner.x + inner.width.saturating_sub(confirm_w + gap + dismiss_w) / 2;

    let confirm_rect = Rect {
        x: btn_x,
        y: btn_y,
        width: confirm_w,
        height: 1,
    };
    let dismiss_rect = Rect {
        x: btn_x + confirm_w + gap,
        y: btn_y,
        width: dismiss_w,
        height: 1,
    };

    let confirm_focused = app.focus == Target::Button(ButtonId::ResumeSessionConfirm);
    let dismiss_focused = app.focus == Target::Button(ButtonId::ResumeSessionDismiss);

    let confirm_style = if confirm_focused {
        theme::selected()
    } else {
        theme::accent()
    };
    let dismiss_style = if dismiss_focused {
        theme::selected()
    } else {
        theme::dim()
    };

    app.layout
        .borrow_mut()
        .register(Target::Button(ButtonId::ResumeSessionConfirm), confirm_rect);
    app.layout
        .borrow_mut()
        .register(Target::Button(ButtonId::ResumeSessionDismiss), dismiss_rect);

    frame.render_widget(
        Paragraph::new(Span::styled("[ Enter / Y ] Resume", confirm_style)),
        confirm_rect,
    );
    frame.render_widget(
        Paragraph::new(Span::styled("[ Esc / N ] Start fresh", dismiss_style)),
        dismiss_rect,
    );
}

fn draw_overlay(frame: &mut Frame, app: &App) {
    app.layout
        .borrow_mut()
        .push_scope(active_popup_area(app, frame.area()));
    if matches!(app.overlay, Overlay::Palette) {
        draw_palette(frame, app);
        return;
    }
    if app.overlay == Overlay::Configs {
        let area = configs_area(app, frame.area());
        super::profile_config::draw(frame, app, area);
        return;
    }
    if matches!(app.overlay, Overlay::AddFallback) {
        draw_add_fallback(frame, app);
        return;
    }
    if let Overlay::Choice(kind) = app.overlay {
        draw_choice(frame, app, kind);
        return;
    }

    if let Overlay::ResumeSession(ref session) = app.overlay {
        draw_resume_session(frame, app, session);
        return;
    }
    let area = active_popup_area(app, frame.area());
    cover(frame, area);
    let title = match &app.overlay {
        Overlay::Help => " Shortcuts ",
        Overlay::Memories { .. } => " Memory ",
        Overlay::Block { .. } => " Detail ",
        Overlay::ResumeSession(_) => " Open last app view session ",
        Overlay::Choice(_)
        | Overlay::Palette
        | Overlay::AddFallback
        | Overlay::Configs
        | Overlay::None => " ",
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
    let lines = popup_lines(app, inner_width);
    let body = draw_see_more(
        frame,
        app,
        body,
        app.scrolls.popup,
        lines.len(),
        ButtonId::SeeMorePopup,
        2,
    );
    let title = if app.focus == Target::Button(ButtonId::SeeMorePopup) {
        format!("{} · focused ", title.trim())
    } else {
        title.to_string()
    };
    frame.render_widget(
        Paragraph::new(lines)
            .style(theme::card_text())
            .block(theme::card(&title))
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
    app.layout
        .borrow_mut()
        .register(Target::CloseOverlay, close);
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
    let items = app.palette_items();
    let mut rows = Vec::new();
    let mut group = "";
    for (index, item) in items.iter().enumerate() {
        if group != item.category {
            if !rows.is_empty() {
                rows.push((None, String::new()));
            }
            group = &item.category;
            rows.push((None, group.to_owned()));
        }
        let hint = if !item.enabled {
            item.disabled_reason.clone()
        } else if item.shortcut.is_empty() {
            item.description.clone()
        } else {
            format!("{} · {}", item.shortcut, item.description)
        };
        rows.push((
            Some(index),
            format!(
                "{}{} · {hint}",
                if index == app.palette_sel {
                    "▸ "
                } else {
                    "  "
                },
                item.label
            ),
        ));
    }
    let room = inner.height.saturating_sub(1) as usize;
    let selected_row = rows
        .iter()
        .position(|(index, _)| *index == Some(app.palette_sel))
        .unwrap_or(0);
    let start = selected_row.saturating_sub(room.saturating_sub(1));
    for (row, (index, label)) in rows.iter().skip(start).take(room).enumerate() {
        let rect = Rect::new(inner.x, inner.y + 1 + row as u16, inner.width, 1);
        let style = match index {
            None => theme::accent().add_modifier(Modifier::BOLD),
            Some(index) if !items[*index].enabled => theme::card_dim(),
            Some(index) if *index == app.palette_sel => theme::selected(),
            _ => theme::card_text(),
        };
        if let Some(index) = index {
            app.layout
                .borrow_mut()
                .register(Target::Choice(*index), rect);
        }
        frame.render_widget(
            Paragraph::new(fit(label, inner.width as usize)).style(style),
            rect,
        );
    }
}

fn draw_choice(frame: &mut Frame, app: &App, kind: ChoiceKind) {
    let area = popup_area(frame.area());
    cover(frame, area);
    let title = match kind {
        ChoiceKind::Provider => format!(" {} provider ", app.defaults_role.label()),
        ChoiceKind::Model => format!(" {} model ", app.defaults_role.label()),
        ChoiceKind::IntelDay => " news cycle day ".into(),
        ChoiceKind::Investigation => " switch investigation ".into(),
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
            ChoiceKind::Investigation => "No investigations yet.",
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
    for (index, item) in app.choice_items.iter().enumerate().skip(start).take(
        super::components::ListGeometry::capacity(height as usize, 1),
    ) {
        app.layout.borrow_mut().register(
            Target::Choice(index),
            Rect {
                x: inner.x,
                y: y + (index - start) as u16 * 2,
                width: inner.width,
                height: 1,
            },
        );
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
                y: y + (index - start) as u16 * 2,
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
    app.layout
        .borrow_mut()
        .register(Target::CloseOverlay, close);
    frame.render_widget(Paragraph::new(" close ").style(theme::card_accent()), close);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_geometry_stamp_tracks_targets_and_snapshot_order() {
        use super::super::topology::TopologyState;
        use argos_osint_core::brain_topology::{
            CycleScope, TopologyNode, TopologyNodeKind, TopologySeed, TopologySnapshot,
        };

        let baseline = TopologyState::default();
        let original = topology_geometry_stamp(&baseline);
        let mutations: [fn(&mut TopologyState); 14] = [
            |s| s.seed = Some(TopologySeed::Claim("claim-a".into())),
            |s| s.depth = 3,
            |s| s.cycle_scope = CycleScope::ThisCycle("cycle-a".into()),
            |s| s.filter_actor = false,
            |s| s.zoom_percent = 150,
            |s| s.pan = (4, 2),
            |s| s.viewport_center_id = Some("claim-a".into()),
            |s| s.active_panel_tab = 1,
            |s| s.inspector_tab = 1,
            |s| s.inspector_scroll = 8,
            |s| s.related_scroll = 2,
            |s| s.related_sel = 1,
            |s| s.selected_node_id = Some("claim-a".into()),
            |s| s.selected_edge_id = Some("edge-a".into()),
        ];
        for mutate in mutations {
            let mut changed = baseline.clone();
            mutate(&mut changed);
            assert_ne!(topology_geometry_stamp(&changed), original);
        }

        let node = |id: &str| TopologyNode {
            id: id.into(),
            kind: TopologyNodeKind::Claim,
            label: id.into(),
            detail: String::new(),
            subtype: String::new(),
            status: String::new(),
            is_root: false,
            article_id: String::new(),
            run_id: String::new(),
            published_at: String::new(),
            cycle: None,
            member_ids: Vec::new(),
            evidence_count: 0,
            passages: Vec::new(),
            min_hops: 0,
            revision: String::new(),
            predecessor: None,
            topology_seed: None,
        };
        let mut changed = baseline;
        changed.snapshot = Some(TopologySnapshot {
            root_id: "claim-a".into(),
            nodes: vec![node("claim-a"), node("claim-b")],
            ..Default::default()
        });
        let ordered = topology_geometry_stamp(&changed);
        changed.snapshot.as_mut().unwrap().nodes.reverse();
        assert_ne!(topology_geometry_stamp(&changed), ordered);
        let reordered = topology_geometry_stamp(&changed);
        changed.snapshot.as_mut().unwrap().memory_changed_seq += 1;
        assert_ne!(topology_geometry_stamp(&changed), reordered);
        let revision = topology_geometry_stamp(&changed);
        changed.snapshot.as_mut().unwrap().nodes[0].label = "Long changed node label".into();
        assert_ne!(topology_geometry_stamp(&changed), revision);
    }

    #[test]
    fn intel_topology_records_keep_wrapped_rows_and_gap_targets_separate() {
        let rows = intel_topology_rows(
            [
                "東京 harbor authority awarded a long logistics contract".to_string(),
                "Another retained claim".to_string(),
            ],
            18,
            Target::IntelTopologyClaim,
        );
        assert_eq!(rows.records.len(), 2);
        let (first, first_target) = &rows.records[0];
        let (second, second_target) = &rows.records[1];
        assert!(
            first.len() > 1,
            "a wrapped record must have one whole hit range"
        );
        assert_eq!(*first_target, Target::IntelTopologyClaim(0));
        assert_eq!(*second_target, Target::IntelTopologyClaim(1));
        assert_eq!(second.start, first.end + 1);
        assert!(rows.lines[first.end].to_string().is_empty());
        assert_eq!(rows.lines.len(), second.end);
        let clipped = intersect_abs(
            Rect::new(0, 5, 20, 3),
            abs_rect(1, 4, 18, first.len() as u16),
        )
        .unwrap();
        assert_eq!(clipped.0.y, 5);
        assert_eq!(clipped.1, 1);
    }

    #[test]
    fn intel_topology_records_preserve_wide_unicode_and_trailing_words() {
        let text = "Harbor Ledger · reported: The Bay Authority awarded the harbor contract to Aster Logistics. Unicode 東京 keeps the complete wrapped record actionable.";
        for width in [18, 33, 38, 58] {
            let rows = intel_topology_rows([text.to_string()], width, Target::IntelTopologyClaim);
            assert!(
                rows.lines.iter().all(|line| line.width() <= width),
                "every painted row must fit the terminal cells at width {width}"
            );
            let painted = rows
                .lines
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(
                painted.split_whitespace().collect::<Vec<_>>().join(" "),
                format!("• {text}"),
                "wrapping must retain each word of the actionable record"
            );
            assert_eq!(rows.records[0].0, 0..rows.lines.len());
        }
    }

    #[test]
    fn dense_atlas_pointer_rows_preserve_offsets_and_reject_borders() {
        let area = Rect::new(4, 8, 30, 6);
        assert_eq!(atlas_row_at(area, 10, 8), None);
        for (row, y) in (9..13).enumerate() {
            assert_eq!(atlas_row_at(area, 10, y), Some(10 + row));
        }
        assert_eq!(atlas_row_at(area, 10, 13), None);
    }

    #[test]
    fn home_dense_items_keep_section_boundaries() {
        let rows = home_rows(Rect::new(0, 0, 120, 40), 0);
        let apps: Vec<_> = rows.iter().filter(|row| row.target.is_some()).collect();
        assert_eq!(apps.len(), 9);
        for pair in apps[..4].windows(2).chain(apps[4..].windows(2)) {
            assert_eq!(pair[1].y, pair[0].y + 1);
        }
        assert_eq!(apps[4].y, apps[3].y + 3);
    }
    use argos_osint_core::osint::ToolResult;
    use argos_osint_core::recon::{Binding, Call, Directive, PickRecord, PlanCall};

    #[test]
    fn coverage_only_counts_explicit_assessments() {
        let plan = Plan {
            directives: vec![
                Directive {
                    id: "d1".into(),
                    ..Default::default()
                },
                Directive {
                    id: "d2".into(),
                    ..Default::default()
                },
                Directive {
                    id: "d3".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let summary = coverage_summary(
            &plan,
            "D1: met [call-a]\nD2: partly met [call-b]\nD3: still being assessed",
        );
        assert!(summary.contains("1 met · 1 partial · 0 not met"));
        assert!(summary.contains("D3 unassessed"));
    }

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
            report_mode: "explain".into(),
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
                Binding {
                    kind: "url".into(),
                    value: "https://example.test/brain-article".into(),
                    evidence_id: "brain:mem-1".into(),
                    step_id: String::new(),
                    qualifier: String::new(),
                    inferred: false,
                    unverified: false,
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
            recon_model: "openrouter / openai/gpt-4.1".into(),
            synthesis_model: "openrouter / openai/gpt-4.1".into(),
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
                .starts_with("◆ Plan · 3 directives · 2 tools · Explain"),
            "{}",
            block.title
        );
        assert!(block.body.contains("Collection strategy"));
        assert!(block.body.contains("○ unassessed"));
        assert!(!block.body.contains("Tool picker: decisions"));
        let details = question_plan_lines(&run, &plan, &calls).join("\n");
        for needle in [
            "Report mode: Explain",
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
            "From Brain: url https://example.test/brain-article",
            "From the question: handle janeroe (twitter) · named in d2, unverified",
            "s1 firecrawl_search: rules found 1; Recon model added 0",
            "Fallback requests:",
        ] {
            assert!(details.contains(needle), "missing {needle:?} in\n{details}");
        }
        assert!(
            details.contains("p=0.87"),
            "picker confidence remains in details"
        );
        assert!(
            !block.body.contains("p=0.87"),
            "picker confidence stays out of the main Plan"
        );
        assert!(
            !details.contains("Jane Roe role"),
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
            .position(|row| {
                matches!(
                    row.kind,
                    HomeKind::Logo(_) | HomeKind::Heading("ARGOS OSINT")
                )
            })
            .unwrap();
        assert!(
            logo_at <= 7,
            "top pad should be half the old even split, got {logo_at}"
        );
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
        assert_eq!(apps, [ModuleId::Intel, ModuleId::Atlas, ModuleId::Brain]);
        assert!(rows.iter().any(|row| {
            (matches!(row.kind, HomeKind::Heading("ARGOS OSINT"))
                || matches!(&row.kind, HomeKind::Logo(_)))
                && row.center
        }));
    }

    #[test]
    fn home_composer_and_all_content_centered_vertically_and_horizontally() {
        let area = Rect::new(0, 0, 120, 40);
        let areas = home_composer_areas(area);

        // 1. Compose box is horizontally centered
        assert_eq!(areas.box_rect.x, (area.width - areas.box_rect.width) / 2);
        assert_eq!(areas.guidance.x, areas.box_rect.x);
        assert_eq!(areas.metadata.x, areas.box_rect.x);
        assert_eq!(areas.metadata.width, areas.box_rect.width);

        // 2. Compose box is vertically positioned near the screen center
        let box_mid_y = areas.box_rect.y + areas.box_rect.height / 2;
        let screen_mid_y = area.height / 2;
        assert!(
            box_mid_y < screen_mid_y && areas.box_rect.bottom() < home_layout_metrics(area).apps_y,
            "composer must precede the spaced launcher within the centered content block"
        );

        // 3. Balanced top and bottom padding across entire page
        let rows = home_rows(area, 0);
        let first_logo_row = rows
            .iter()
            .find(|r| matches!(r.kind, HomeKind::Logo(_)))
            .expect("logo row should exist in wide mode");
        let last_app_row = rows
            .iter()
            .rfind(|r| r.target.is_some())
            .expect("last app row should exist");

        let top_pad = first_logo_row.y - area.y;
        let bottom_pad = (area.y + area.height).saturating_sub(last_app_row.y + 1);
        assert!(
            (top_pad as i32 - bottom_pad as i32).abs() <= 2,
            "top padding ({top_pad}) and bottom padding ({bottom_pad}) should be balanced"
        );

        // 4. Content below composer (apps column) is horizontally centered
        let column = rows
            .iter()
            .filter(|r| !r.center)
            .map(home_row_width)
            .max()
            .unwrap_or(0) as u16;
        let expected_left = area.x + (area.width - column) / 2;
        assert!(expected_left > 0);
        assert_eq!(expected_left, (area.width - column) / 2);

        // 5. Verify narrow terminal mode (80x24) centers without overflowing
        let narrow_area = Rect::new(0, 0, 80, 24);
        let narrow_metrics = home_layout_metrics(narrow_area);
        let narrow_rows = home_rows(narrow_area, 0);
        assert!(narrow_metrics.top_pad <= 6);
        let narrow_last_y = narrow_rows.last().map(|r| r.y).unwrap_or(0);
        let narrow_bottom_pad = narrow_area.height.saturating_sub(narrow_last_y + 1);
        assert!(
            (narrow_metrics.top_pad as i32 - narrow_bottom_pad as i32).abs() <= 1,
            "narrow mode top and bottom padding should be balanced"
        );
        assert!(
            narrow_last_y < narrow_area.height,
            "all rows must fit within height 24, last y was {narrow_last_y}"
        );

        // 6. Cursor blink is available
        let _ = cursor_blink_visible();

        // 7. Verify recon_composer_areas is full width
        let recon_area = Rect::new(0, 30, 100, 4);
        let recon_areas = recon_composer_areas(recon_area);
        assert_eq!(recon_areas.box_rect.x, recon_area.x);
        assert_eq!(recon_areas.box_rect.width, recon_area.width);
        assert_eq!(recon_areas.guidance.width, 0);
    }

    #[test]
    fn atlas_source_anchors_are_not_labeled_deleted_origin() {
        let insight = recon::InsightView {
            metadata: None,
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
