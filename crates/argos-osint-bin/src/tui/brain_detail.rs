//! Brain memory detail: navigation history, the Related list, and the shared
//! layout of the Claim/Recon detail view (graph above, Related left, Summary
//! right). Drawing of the path graph and summary stays in `graph.rs`.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use argos_osint_core::brain::Memory;
use argos_osint_core::related_memories::{RelatedMemory, RelationKind};

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

/// Related memories of the open detail memory.
#[derive(Clone, Debug, Default)]
pub struct RelatedView {
    pub items: Vec<RelatedMemory>,
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
    pub related_sel: usize,
    pub related_scroll: u16,
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
        self.related.request
    }

    /// Accept a related result only for the current request and memory.
    pub fn finish_related(
        &mut self,
        request: u64,
        memory_id: &str,
        outcome: Result<Vec<RelatedMemory>, String>,
    ) -> bool {
        if request != self.related.request || self.memory_id() != Some(memory_id) {
            return false;
        }
        match outcome {
            Ok(items) => {
                self.related.items = items;
                self.related.state = RelatedState::Ready;
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

    pub fn selected_related(&self) -> Option<&RelatedMemory> {
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
        Target::RelatedRow(_) => DetailPane::Related,
        Target::DetailSummary => DetailPane::Summary,
        _ => DetailPane::Path,
    }
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
    Header(&'static str),
    Title(usize),
    Reason(usize),
    Note(String),
}

/// The Related section's lines, in paint order. Hit-testing uses the same list.
pub fn related_rows(view: &RelatedView) -> Vec<RelRow> {
    match &view.state {
        RelatedState::Idle | RelatedState::Loading => {
            return vec![RelRow::Note("Loading related memories…".into())]
        }
        RelatedState::Failed(err) => {
            return vec![RelRow::Note(format!("Related memories unavailable: {err}"))]
        }
        RelatedState::Ready if view.items.is_empty() => {
            return vec![RelRow::Note("No related memories".into())]
        }
        RelatedState::Ready => {}
    }
    let mut rows = Vec::new();
    for (kind, header) in [
        (RelationKind::Explicit, "Linked"),
        (RelationKind::Similar, "Similar · not evidence"),
    ] {
        let mut first = true;
        for (index, item) in view.items.iter().enumerate() {
            if item.kind != kind {
                continue;
            }
            if first {
                rows.push(RelRow::Header(header));
                first = false;
            }
            rows.push(RelRow::Title(index));
            rows.push(RelRow::Reason(index));
        }
    }
    rows
}

/// Related item under a point, or `None` for headers, notes, and misses.
pub fn related_at(view: &RelatedView, area: Rect, x: u16, y: u16) -> Option<usize> {
    let content = inner(area);
    if !contains(content, x, y) {
        return None;
    }
    let row = view.scroll as usize + (y - content.y) as usize;
    match related_rows(view).get(row)? {
        RelRow::Title(index) | RelRow::Reason(index) => Some(*index),
        _ => None,
    }
}

pub fn related_scroll_max(view: &RelatedView, area: Rect) -> u16 {
    let rows = related_rows(view).len();
    rows.saturating_sub(inner(area).height as usize) as u16
}

/// Scroll so the selected item's two lines are visible.
pub fn reveal_related(view: &mut RelatedView, area: Rect) {
    let height = inner(area).height.max(1) as usize;
    let rows = related_rows(view);
    let Some(top) = rows.iter().position(|row| *row == RelRow::Title(view.sel)) else {
        return;
    };
    // Keep the group header visible for the first item of a group.
    let start = if top > 0 && matches!(rows[top - 1], RelRow::Header(_)) {
        top - 1
    } else {
        top
    };
    let bottom = top + 1;
    let scroll = view.scroll as usize;
    if start < scroll {
        view.scroll = start as u16;
    } else if bottom >= scroll + height {
        view.scroll = (bottom + 1).saturating_sub(height) as u16;
    }
}

pub fn draw_related(frame: &mut Frame, app: &App, area: Rect) {
    let view = &app.brain_detail.related;
    let focused = matches!(app.focus, Target::RelatedRow(_));
    let width = inner(area).width as usize;
    let lines: Vec<Line> = related_rows(view)
        .into_iter()
        .map(|row| match row {
            RelRow::Header(text) => Line::from(Span::styled(fit(text, width), theme::accent())),
            RelRow::Note(text) => Line::from(Span::styled(fit(&text, width), theme::dim())),
            RelRow::Title(index) => {
                let item = &view.items[index];
                let mark = match item.kind {
                    RelationKind::Explicit => "⇄",
                    RelationKind::Similar => "≈",
                };
                let style = if index == view.sel && focused {
                    theme::selected()
                } else if index == view.sel {
                    theme::text().add_modifier(ratatui::style::Modifier::BOLD)
                } else {
                    theme::text()
                };
                Line::from(Span::styled(
                    fit(&format!("{mark} {}", item.title), width),
                    style,
                ))
            }
            RelRow::Reason(index) => {
                let item = &view.items[index];
                Line::from(Span::styled(
                    fit(&format!("  {} · {}", item.reason, item.provenance), width),
                    theme::muted(),
                ))
            }
        })
        .collect();
    let title = match view.state {
        RelatedState::Ready if !view.items.is_empty() => {
            format!(" related · {} ", view.items.len())
        }
        _ => " related ".to_string(),
    };
    let title = if focused {
        format!("{}· focused ", title)
    } else {
        title
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(panel(&title))
            .scroll((view.scroll, 0)),
        area,
    );
}

/// One-line strip above the graph: Back, mode, title, history depth.
pub fn draw_nav(frame: &mut Frame, app: &App, areas: &DetailAreas, claim: bool) {
    let back_style = if app.focus == Target::Button(ButtonId::BrainDetailBack) {
        theme::selected()
    } else {
        theme::accent()
    };
    frame.render_widget(
        Paragraph::new(Span::styled("‹ Back", back_style)),
        areas.back,
    );
    let rest = Rect {
        x: areas.nav.x + areas.back.width,
        width: areas.nav.width.saturating_sub(areas.back.width),
        ..areas.nav
    };
    let title = app
        .brain_detail
        .memory
        .as_ref()
        .map(|memory| memory.text.lines().next().unwrap_or("").to_string())
        .unwrap_or_default();
    let depth = app.brain_detail.history.len();
    let back_to = if depth == 0 {
        "memories".to_string()
    } else {
        format!("{depth} back")
    };
    let text = format!(
        "{} · {}  ({})",
        if claim { "Claim path" } else { "Recon path" },
        title,
        back_to
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            fit(&text, rest.width as usize),
            theme::muted(),
        )),
        rest,
    );
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
