//! Topology "Scope summary": markdown explanation of how the nodes in the
//! current scope connect. State, text selection and drawing live here; the
//! background plan/generate worker is in `app.rs` and the freshness rules in
//! `argos_osint_core::topology_summary_run`. Drawing reads this state only:
//! no storage, network or model work happens during a draw.
use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use argos_osint_core::brain_topology::{TopologySeed, TopologySnapshot};
use argos_osint_core::topology_summary::{deterministic_summary, ScopeFilters, ScopeSpec};

use super::app::{App, ButtonId, Target};
use super::components;
use super::theme::{self, panel, BORDER, WARN};
use super::ui::draw_button;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SummaryPhase {
    /// Deterministic text only; no model run has been decided yet.
    #[default]
    Idle,
    /// The Summarization role is writing; deterministic (or saved, stale) text shows.
    Writing,
    /// The AI narrative for exactly this scope and graph version.
    Ready,
    /// A saved narrative that is older than the graph; a refresh is pending.
    Stale,
    /// The last run failed; the deterministic or saved text stays.
    Failed,
    /// No Summarization route; the deterministic text stays.
    Unavailable,
}

impl SummaryPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Idle => "deterministic",
            Self::Writing => "writing…",
            Self::Ready => "AI summary",
            Self::Stale => "stale",
            Self::Failed => "failed",
            Self::Unavailable => "no model",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedText {
    pub text: String,
    pub created_at: String,
    pub fresh: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ScopeSummaryState {
    pub scope_key: String,
    pub graph_version: String,
    /// Digest the current plan/run belongs to; results for another digest are
    /// saved by the worker but never displayed.
    pub digest: String,
    pub phase: SummaryPhase,
    pub deterministic: String,
    pub caveat: String,
    pub headline: String,
    pub saved: Option<SavedText>,
    pub notice: String,
    pub scroll: usize,
    pub request: u64,
    pub retry_after: Option<Instant>,
    /// Failure card (the Brain graph_explanation card) for a failed run.
    pub failure: Option<super::summary_card::SummaryFailure>,
    pub details_open: bool,
}

impl ScopeSummaryState {
    /// Scope spec for the visible, filtered neighborhood.
    pub fn spec(
        seed: &TopologySeed,
        depth: usize,
        cycle: &argos_osint_core::brain_topology::CycleScope,
        filters: [bool; 6],
    ) -> ScopeSpec {
        ScopeSpec {
            seed_key: seed.key(),
            depth,
            cycle: cycle.clone(),
            filters: ScopeFilters::from_array(filters),
        }
    }

    /// Recompute the deterministic text instantly for an accepted snapshot.
    pub fn set_deterministic(&mut self, snapshot: &TopologySnapshot, spec: &ScopeSpec) {
        let summary = deterministic_summary(snapshot, spec);
        self.caveat = summary.caveat_line();
        self.headline = format!(
            "{} nodes · {} links · {}",
            summary.node_count, summary.edge_count, self.caveat
        );
        self.deterministic = summary.to_text();
    }

    /// Markdown shown in the panel for the current phase.
    pub fn body_markdown(&self) -> String {
        let mut out = String::new();
        if !self.notice.is_empty() && self.failure.is_none() {
            out.push_str(&format!("*{}*\n\n", self.notice));
        }
        match (&self.saved, self.phase) {
            (Some(saved), SummaryPhase::Ready) => out.push_str(&saved.text),
            (Some(saved), _) => {
                out.push_str("*Earlier summary (stale).*\n\n");
                out.push_str(&saved.text);
            }
            (None, _) => out.push_str(&self.deterministic),
        }
        out
    }

    pub fn can_retry(&self) -> bool {
        matches!(self.phase, SummaryPhase::Failed | SummaryPhase::Stale)
    }

    pub fn throttle_note(retry_in: Duration) -> String {
        format!(
            "The graph changed; a new summary is due in {} min. Showing the saved one",
            retry_in.as_secs().div_ceil(60).max(1)
        )
    }
}

pub fn lines(state: &ScopeSummaryState, width: u16) -> Vec<Line<'static>> {
    super::graph::summary_lines(&state.body_markdown(), width.max(1) as usize)
}

/// Rows the body scrolls: failure details (when expanded) above the text.
fn content_lines(state: &ScopeSummaryState, width: u16) -> Vec<Line<'static>> {
    let mut all = Vec::new();
    if state.phase == SummaryPhase::Failed && state.details_open {
        if let Some(failure) = &state.failure {
            all.extend(
                failure
                    .details
                    .iter()
                    .map(|d| Line::styled(d.clone(), theme::dim())),
            );
            all.push(Line::raw(""));
        }
    }
    all.extend(lines(state, width));
    all
}

/// Scroll the summary body by exactly the extent the last draw measured.
/// `isize::MIN` / `isize::MAX` jump to Home / End.
pub fn scroll(app: &mut App, delta: isize) -> bool {
    let area = app
        .layout
        .borrow()
        .entries
        .iter()
        .find(|entry| entry.target == Target::TopologySummary)
        .map(|entry| entry.rect);
    let Some(area) = area else {
        return false;
    };
    let total = content_lines(&app.topology_state.summary, area.width).len();
    let max = total.saturating_sub(area.height as usize);
    let scroll = &mut app.topology_state.summary.scroll;
    *scroll = scroll.saturating_add_signed(delta).min(max);
    true
}

/// Findings that used to live in the two-row strip under the graph.
fn findings(app: &App, outside: usize, filtered: usize) -> String {
    let state = &app.topology_state;
    if let Some(error) = &app.topology_error {
        return format!(
            "{} · {error}",
            if state.snapshot.is_some() {
                "Retained snapshot"
            } else {
                "Snapshot unavailable"
            }
        );
    }
    if app.topology_loading {
        return if state.snapshot.is_some() {
            "Refreshing retained snapshot…".into()
        } else {
            "Loading seeded neighborhood…".into()
        };
    }
    if outside > 0 || filtered > 0 {
        return if outside > 0 {
            format!("{outside} outside viewport · {filtered} filtered · Shift+arrows pan")
        } else {
            format!("{filtered} filtered")
        };
    }
    String::new()
}

fn footer_text(app: &App, outside: usize, filtered: usize) -> String {
    let state = &app.topology_state;
    let found = findings(app, outside, filtered);
    let caveat = &state.summary.caveat;
    match (found.is_empty(), caveat.is_empty()) {
        (true, true) => String::new(),
        (true, false) => caveat.clone(),
        (false, true) => found,
        (false, false) => format!("{found} · {caveat}"),
    }
}

/// One-line headline used under the graph in Stacked mode.
pub fn draw_headline(frame: &mut Frame, app: &App, area: Rect, outside: usize, filtered: usize) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let state = &app.topology_state;
    let found = findings(app, outside, filtered);
    let text = if found.is_empty() {
        match state.summary.phase {
            SummaryPhase::Idle | SummaryPhase::Ready => {
                format!("Summary ▸ {}", state.summary.headline)
            }
            phase => format!("Summary ▸ {} · {}", phase.label(), state.summary.headline),
        }
    } else {
        found
    };
    let warn = app.topology_error.is_some();
    frame.render_widget(
        Paragraph::new(components::clip_text(&text, area.width as usize)).style(if warn {
            Style::default().fg(WARN)
        } else {
            theme::muted()
        }),
        area,
    );
}

/// Bordered panel: markdown body (independently scrollable), a scroll
/// indicator in the top border, and a one-row footer with the caveat line.
/// A failed run puts the shared failure card at the top of the panel.
pub fn draw_panel(frame: &mut Frame, app: &App, area: Rect, outside: usize, filtered: usize) {
    let state = &app.topology_state;
    let summary = &state.summary;
    let focused = app.focus == Target::TopologySummary;
    let block =
        panel(&format!(" Scope summary · {} ", summary.phase.label())).border_style(if focused {
            theme::accent()
        } else {
            Style::default().fg(BORDER)
        });
    let inner = block.inner(area);
    if inner.width < 6 || inner.height < 2 {
        frame.render_widget(block, area);
        return;
    }
    let content = Rect::new(inner.x + 1, inner.y, inner.width - 2, inner.height);
    let card = match (&summary.failure, summary.phase) {
        (Some(failure), SummaryPhase::Failed) => Some((
            failure,
            super::summary_card::height_with(failure, summary.details_open, content, true),
        )),
        _ => None,
    };
    let card_h = card.map_or(0, |(_, h)| h);
    let rest = Rect::new(
        content.x,
        content.y + card_h,
        content.width,
        content.height.saturating_sub(card_h),
    );
    let footer_rows = u16::from(rest.height >= 2);
    let body = Rect::new(
        rest.x,
        rest.y,
        rest.width,
        rest.height.saturating_sub(footer_rows),
    );
    let footer = Rect::new(rest.x, rest.bottom().saturating_sub(1), rest.width, 1);
    let all = content_lines(summary, body.width);
    let room = body.height as usize;
    let offset = summary.scroll.min(all.len().saturating_sub(room));
    let block = if room > 0 && all.len() > room {
        let end = (offset + room).min(all.len());
        let indicator = format!(
            " {}{}–{}/{}{} ",
            if offset > 0 { "▲ " } else { "" },
            offset + 1,
            end,
            all.len(),
            if end < all.len() { " ▼" } else { "" }
        );
        block.title_top(Line::from(indicator).right_aligned())
    } else {
        block
    };
    frame.render_widget(block, area);
    if let Some((failure, _)) = card {
        super::summary_card::draw_with(frame, app, failure, content, summary.details_open, true);
    }
    if room > 0 {
        app.layout
            .borrow_mut()
            .register(Target::TopologySummary, body);
        let visible: Vec<_> = all.into_iter().skip(offset).take(room).collect();
        frame.render_widget(Paragraph::new(visible), body);
    }
    if footer_rows == 0 {
        return;
    }
    let mut text_width = footer.width as usize;
    // The card carries its own Retry; the plain button is for stale text.
    if summary.can_retry() && card.is_none() {
        let label = "[Retry]";
        let width = label.len() as u16;
        if footer.width > width + 8 {
            let rect = Rect::new(footer.right() - width, footer.y, width, 1);
            draw_button(frame, app, ButtonId::TopologySummaryRetry, label, rect);
            text_width = footer.width.saturating_sub(width + 1) as usize;
        }
    }
    let text = footer_text(app, outside, filtered);
    let warn = app.topology_error.is_some() || summary.phase == SummaryPhase::Failed;
    frame.render_widget(
        Paragraph::new(components::clip_text(&text, text_width)).style(if warn {
            Style::default().fg(WARN)
        } else {
            theme::muted().add_modifier(Modifier::empty())
        }),
        Rect::new(footer.x, footer.y, text_width as u16, 1),
    );
}
