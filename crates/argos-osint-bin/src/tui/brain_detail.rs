//! Brain memory detail: navigation history, the Related list, and the shared
//! layout of the Claim/Recon detail view (graph above, Related left, Summary
//! right). Drawing of the path graph and summary stays in `graph.rs`.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use argos_osint_core::brain::Memory;

use super::app::{App, ButtonId, Target};
use super::theme::{self, panel};
use super::ui::{contains, fit};

/// Bounded detail-navigation history.
pub const HISTORY_CAP: usize = 32;
/// Each lower column needs this many cells before Related and Summary sit side by side.
pub const MIN_COLUMN: u16 = 34;

#[derive(Clone, Debug, Default, PartialEq)]
pub enum RelatedState {
    #[default]
    Idle,
    Loading,
    Ready,
    Failed(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RelatedTab {
    #[default]
    Claims,
    Threads,
    Articles,
    Actors,
    Sources,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssociationTarget {
    Claim(String),
    ActorGroup(String),
    Thread(String),
    Article {
        run_id: String,
        article_id: String,
        revision: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssociationItem {
    pub key: String,
    pub title: String,
    pub reason: String,
    pub target: AssociationTarget,
}

impl RelatedTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Claims => "Claims",
            Self::Threads => "Threads",
            Self::Articles => "Articles",
            Self::Actors => "Actors",
            Self::Sources => "Sources",
        }
    }
    pub fn index(self) -> usize {
        match self {
            Self::Claims | Self::Actors => 0,
            Self::Threads | Self::Sources => 1,
            Self::Articles => 2,
        }
    }
}

/// Related memories of the open detail memory.
#[derive(Clone, Debug, Default)]
pub struct RelatedView {
    pub items: Vec<AssociationItem>,
    #[cfg(test)]
    pub articles: super::related_articles::ArticleRelatedView,
    pub tab: RelatedTab,
    pub cursors: [(usize, u16); 3],
    pub page_sizes: [usize; 3],
    pub selected_keys: [Option<String>; 3],
    pub state: RelatedState,
    pub sel: usize,
    pub scroll: u16,
    /// Monotonic request id; late results with another id are dropped.
    pub request: u64,
}

/// Which lower/upper section of the detail view has focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailPane {
    Path,
    Related,
    Summary,
}

/// What Back restores.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetailSnapshot {
    pub memory_id: String,
    pub compact_page: usize,
    pub related_sel: usize,
    pub related_scroll: u16,
    pub tab: RelatedTab,
    pub cursors: [(usize, u16); 3],
    pub page_sizes: [usize; 3],
    pub selected_keys: [Option<String>; 3],
    pub path_scroll: u16,
    pub summary_scroll: u16,
    pub focus: Target,
}

/// State of the open memory detail, independent of the Brain list selection.
#[derive(Clone, Debug, Default)]
pub struct BrainDetail {
    /// The memory whose graph, Related list, and Summary are shown.
    pub memory: Option<Memory>,
    pub related: RelatedView,
    pub history: Vec<DetailSnapshot>,
}

impl BrainDetail {
    pub fn memory_id(&self) -> Option<&str> {
        self.memory.as_ref().map(|memory| memory.id.as_str())
    }

    pub fn push(&mut self, snapshot: DetailSnapshot) {
        self.history.push(snapshot);
        if self.history.len() > HISTORY_CAP {
            let extra = self.history.len() - HISTORY_CAP;
            self.history.drain(..extra);
        }
    }

    /// Start a new related request; returns its id.
    pub fn begin_related(&mut self, sel: usize, scroll: u16) -> u64 {
        self.related.request += 1;
        self.related.items.clear();
        self.related.state = RelatedState::Loading;
        self.related.sel = sel;
        self.related.scroll = scroll;
        self.related.page_sizes[self.related.tab.index()] = self.related.page_sizes
            [self.related.tab.index()]
        .max(20)
        .max(sel + 1);
        self.related.request
    }

    /// Accept a related result only for the current request and memory.
    pub fn finish_related(
        &mut self,
        request: u64,
        memory_id: &str,
        outcome: Result<Vec<AssociationItem>, String>,
    ) -> bool {
        if request != self.related.request || self.memory_id() != Some(memory_id) {
            return false;
        }
        match outcome {
            Ok(items) => {
                self.related.items = items;
                self.related.state = RelatedState::Ready;
                if let Some(key) = &self.related.selected_keys[self.related.tab.index()] {
                    if let Some(index) = self.related.items.iter().position(|item| &item.key == key)
                    {
                        self.related.sel = index;
                    }
                }
                self.related.page_sizes[self.related.tab.index()] = self.related.page_sizes
                    [self.related.tab.index()]
                .max(20)
                .max(self.related.sel + 1);
                if self.related.sel >= self.related.items.len() {
                    self.related.sel = self.related.items.len().saturating_sub(1);
                }
            }
            Err(err) => {
                self.related.items.clear();
                self.related.state = RelatedState::Failed(err);
                self.related.sel = 0;
            }
        }
        true
    }

    pub fn selected_related(&self) -> Option<&AssociationItem> {
        self.related.items.get(self.related.sel)
    }
}

/// Rectangles of the detail view. `stacked` when Related and Summary do not
/// fit side by side and are stacked (Related above Summary).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetailAreas {
    pub nav: Rect,
    pub back: Rect,
    pub path: Rect,
    pub related: Rect,
    pub summary: Rect,
    pub stacked: bool,
}

pub fn pane_of(focus: Target) -> DetailPane {
    match focus {
        Target::RelatedRow(_)
        | Target::Button(
            ButtonId::BrainRelatedFirst
            | ButtonId::BrainRelatedSecond
            | ButtonId::BrainRelatedThird
            | ButtonId::BrainAssociationsMore,
        ) => DetailPane::Related,
        Target::DetailSummary => DetailPane::Summary,
        Target::Button(button) if super::summary_card::is_card_button(button) => {
            DetailPane::Summary
        }
        _ => DetailPane::Path,
    }
}

pub fn areas_for(app: &App, area: Rect) -> DetailAreas {
    let actor = app
        .brain_detail
        .memory_id()
        .is_some_and(|id| id.starts_with("actor-group:"));
    let mut layout = areas(area, pane_of(app.focus));
    if actor {
        let content = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );
        layout.summary = Rect::default();
        if area.width >= 100 && app.screen.height >= 28 {
            let left = area.width * 65 / 100;
            layout.path = Rect::new(content.x, content.y, left, content.height);
            layout.related = Rect::new(
                content.x + left,
                content.y,
                content.width - left,
                content.height,
            );
        } else {
            layout.path = if pane_of(app.focus) == DetailPane::Related {
                Rect::default()
            } else {
                content
            };
            layout.related = if pane_of(app.focus) == DetailPane::Related {
                content
            } else {
                Rect::default()
            };
        }
    }
    let page = app.compact_pages[super::app::ModuleId::Brain.index()];
    if page > 0 {
        let content = Rect::new(
            area.x,
            area.y + 1,
            area.width,
            area.height.saturating_sub(1),
        );
        layout.path = if page == 1 { content } else { Rect::default() };
        layout.related = if page == 2 { content } else { Rect::default() };
        layout.summary = if page == 3 { content } else { Rect::default() };
    }
    layout
}

pub fn areas(area: Rect, focus: DetailPane) -> DetailAreas {
    let nav = Rect {
        height: 1.min(area.height),
        ..area
    };
    let back = Rect {
        width: 8.min(nav.width),
        ..nav
    };
    let rest = Rect {
        y: area.y + nav.height,
        height: area.height.saturating_sub(nav.height),
        ..area
    };
    let path_h = (rest.height / 2).max(6.min(rest.height));
    let path = Rect {
        height: path_h,
        ..rest
    };
    let lower = Rect {
        y: rest.y + path_h,
        height: rest.height.saturating_sub(path_h),
        ..rest
    };
    if lower.width >= MIN_COLUMN * 2 {
        let left = lower.width / 2;
        return DetailAreas {
            nav,
            back,
            path,
            related: Rect {
                width: left,
                ..lower
            },
            summary: Rect {
                x: lower.x + left,
                width: lower.width - left,
                ..lower
            },
            stacked: false,
        };
    }
    // Stacked: the focused section gets the larger share; both stay visible.
    let related_h = match focus {
        DetailPane::Related => lower.height * 2 / 3,
        DetailPane::Summary => lower.height / 3,
        DetailPane::Path => lower.height / 2,
    }
    .max(3.min(lower.height));
    DetailAreas {
        nav,
        back,
        path,
        related: Rect {
            height: related_h,
            ..lower
        },
        summary: Rect {
            y: lower.y + related_h,
            height: lower.height.saturating_sub(related_h),
            ..lower
        },
        stacked: true,
    }
}

pub fn inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

/// One painted line of the Related section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelRow {
    Title(usize, String),
    Reason(usize, String),
    Note(String),
}

/// The Related section's lines, in paint order. Hit-testing uses the same list.
pub fn related_rows(view: &RelatedView, width: usize) -> Vec<RelRow> {
    match &view.state {
        RelatedState::Idle | RelatedState::Loading => {
            return vec![RelRow::Note(super::anim::loading("Loading associations"))]
        }
        RelatedState::Failed(err) => {
            return vec![RelRow::Note(format!(
                "Associations unavailable: {err} · select tab to retry"
            ))]
        }
        RelatedState::Ready if view.items.is_empty() => {
            return vec![RelRow::Note(format!(
                "No {}",
                view.tab.label().to_lowercase()
            ))]
        }
        RelatedState::Ready => {}
    }
    let mut rows = Vec::new();
    for (index, item) in view
        .items
        .iter()
        .enumerate()
        .take(view.page_sizes[view.tab.index()].max(20))
    {
        if index > 0 {
            rows.push(RelRow::Note(String::new()));
        }
        for text in super::ui::wrap_text(&format!("· {}", item.title), width.max(1)) {
            rows.push(RelRow::Title(index, text));
        }
        for text in super::ui::wrap_text(&format!("  {}", item.reason), width.max(1)) {
            rows.push(RelRow::Reason(index, text));
        }
    }
    rows
}

/// Related item under a point, or `None` for headers, notes, and misses.
fn related_content_area(area: Rect) -> Rect {
    Rect::new(
        area.x,
        area.y.saturating_add(1),
        area.width,
        area.height.saturating_sub(1),
    )
}
pub fn related_at(view: &RelatedView, area: Rect, x: u16, y: u16) -> Option<usize> {
    let content = inner(related_content_area(area));
    if !contains(content, x, y) {
        return None;
    }
    let row = view.scroll as usize + (y - content.y) as usize;
    match related_rows(view, content.width as usize).get(row)? {
        RelRow::Title(index, _) | RelRow::Reason(index, _) => Some(*index),
        _ => None,
    }
}

pub fn related_scroll_max(view: &RelatedView, area: Rect) -> u16 {
    let rows = related_rows(view, inner(related_content_area(area)).width as usize).len();
    rows.saturating_sub(inner(related_content_area(area)).height as usize) as u16
}

/// Scroll so the selected item's two lines are visible.
pub fn reveal_related(view: &mut RelatedView, area: Rect) {
    let height = inner(related_content_area(area)).height.max(1) as usize;
    let rows = related_rows(view, inner(related_content_area(area)).width as usize);
    let Some(top) = rows
        .iter()
        .position(|row| matches!(row,RelRow::Title(index,_) if *index==view.sel))
    else {
        return;
    };
    // Keep the group header visible for the first item of a group.
    let start = top;
    let bottom = rows
        .iter()
        .rposition(
            |row| matches!(row,RelRow::Title(index,_)|RelRow::Reason(index,_) if *index==view.sel),
        )
        .unwrap_or(top);
    if bottom.saturating_sub(start) + 1 > height {
        // When a short split pane cannot fit the whole item, retain its title;
        // opening the expanded pane must not start on an orphaned reason line.
        view.scroll = start as u16;
        return;
    }
    let scroll = view.scroll as usize;
    if start < scroll {
        view.scroll = start as u16;
    } else if bottom >= scroll + height {
        view.scroll = (bottom + 1).saturating_sub(height) as u16;
    }
}

pub fn draw_related(frame: &mut Frame, app: &App, area: Rect) {
    if area.width < 3 || area.height < 3 {
        return;
    }
    let tabs = Rect::new(area.x, area.y, area.width, 1);
    let actor = app
        .brain_detail
        .memory_id()
        .is_some_and(|id| id.starts_with("actor-group:"));
    let labels: &[&str] = if actor {
        &["Claims", "Threads", "Articles"]
    } else {
        &["Actors", "Sources"]
    };
    let buttons = [
        ButtonId::BrainRelatedFirst,
        ButtonId::BrainRelatedSecond,
        ButtonId::BrainRelatedThird,
    ];
    for (index, label) in labels.iter().enumerate() {
        let x = tabs.x + tabs.width * index as u16 / labels.len() as u16;
        let end = tabs.x + tabs.width * (index + 1) as u16 / labels.len() as u16;
        super::ui::draw_button_state(
            frame,
            app,
            buttons[index],
            label,
            Rect::new(x, tabs.y, end - x, 1),
            app.brain_detail.related.tab.index() == index,
        );
    }
    let area = related_content_area(area);
    let view = &app.brain_detail.related;
    let focused = matches!(
        app.focus,
        Target::RelatedRow(_)
            | Target::Button(
                ButtonId::BrainAssociationsMore
                    | ButtonId::BrainRelatedFirst
                    | ButtonId::BrainRelatedSecond
                    | ButtonId::BrainRelatedThird
            )
    );
    if view.items.is_empty() {
        app.layout
            .borrow_mut()
            .register(Target::RelatedRow(view.sel), area);
    }
    let width = inner(area).width as usize;

    let mut inner_area = inner(area);
    let more = view.items.len() > view.page_sizes[view.tab.index()].max(20);
    inner_area.height = inner_area.height.saturating_sub(u16::from(more));
    let mut current_y = inner_area.y;
    for (i, row) in related_rows(view, width).into_iter().enumerate() {
        if i < view.scroll as usize {
            continue;
        }
        if current_y >= inner_area.y + inner_area.height {
            break;
        }
        if let RelRow::Title(index, _) | RelRow::Reason(index, _) = row {
            app.layout.borrow_mut().register(
                Target::RelatedRow(index),
                Rect {
                    x: inner_area.x,
                    y: current_y,
                    width: inner_area.width,
                    height: 1,
                },
            );
        }
        current_y += 1;
    }

    let loading = matches!(view.state, RelatedState::Idle | RelatedState::Loading);
    let lines: Vec<Line> = if loading {
        super::anim::centered_lines(
            "Loading associations",
            theme::dim(),
            inner_area.width,
            inner_area.height,
        )
    } else {
        related_rows(view, width)
            .into_iter()
            .map(|row| match row {
                RelRow::Note(text) => Line::from(Span::styled(fit(&text, width), theme::dim())),
                RelRow::Title(index, text) => {
                    let style = if index == view.sel && focused {
                        theme::selected()
                    } else if index == view.sel {
                        theme::text().add_modifier(ratatui::style::Modifier::BOLD)
                    } else {
                        theme::text()
                    };
                    Line::from(Span::styled(text, style))
                }
                RelRow::Reason(_, text) => Line::from(Span::styled(text, theme::muted())),
            })
            .collect()
    };
    let title = match view.state {
        RelatedState::Ready if !view.items.is_empty() => {
            format!(
                " {} · {} of {} ",
                view.tab.label(),
                view.items
                    .len()
                    .min(view.page_sizes[view.tab.index()].max(20)),
                view.items.len()
            )
        }
        _ => " related ".to_string(),
    };
    let title = if focused {
        format!("{}· focused ", title)
    } else {
        title
    };
    frame.render_widget(
        Paragraph::new(
            lines
                .into_iter()
                .take(view.scroll as usize + inner_area.height as usize)
                .collect::<Vec<_>>(),
        )
        .block(panel(&title))
        .scroll((view.scroll, 0)),
        area,
    );
    if more {
        let content = inner(area);
        if content.height > 0 {
            super::ui::draw_button(
                frame,
                app,
                ButtonId::BrainAssociationsMore,
                "See more",
                Rect::new(content.x, content.y + content.height - 1, content.width, 1),
            );
        }
    }
}

/// One-line strip above the graph: Back, mode, title, history depth.
pub fn draw_nav(frame: &mut Frame, app: &App, areas: &DetailAreas, _claim: bool) {
    let selectors = Rect::new(
        areas.nav.x + areas.back.width,
        areas.nav.y,
        areas.nav.width.saturating_sub(areas.back.width),
        areas.nav.height,
    );
    super::ui::pane_tabs(
        frame,
        app,
        selectors,
        if app
            .brain_detail
            .memory_id()
            .is_some_and(|id| id.starts_with("actor-group:"))
        {
            &["Split", "Graph", "Related"]
        } else {
            &["Split", "Graph", "Related", "Summary"]
        },
    );
    app.layout
        .borrow_mut()
        .register(Target::Button(ButtonId::BrainDetailBack), areas.back);
    let back_style = if app.focus == Target::Button(ButtonId::BrainDetailBack) {
        theme::selected()
    } else {
        theme::accent()
    };
    frame.render_widget(
        Paragraph::new(Span::styled("‹ Back", back_style)),
        areas.back,
    );
}

/// The injected read failure, if any (always `None` outside tests).
pub fn read_fault() -> Option<String> {
    #[cfg(test)]
    {
        testing::read_fault()
    }
    #[cfg(not(test))]
    {
        None
    }
}

/// Test hook: make Brain memory list reads fail on this thread.
#[cfg(test)]
pub mod testing {
    use std::cell::RefCell;

    thread_local! {
        static READ_FAULT: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    pub fn fail_reads(message: Option<&str>) {
        READ_FAULT.with(|fault| *fault.borrow_mut() = message.map(str::to_string));
    }

    pub fn read_fault() -> Option<String> {
        READ_FAULT.with(|fault| fault.borrow().clone())
    }
}
