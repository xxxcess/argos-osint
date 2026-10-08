//! Brain summary failure card: the reason, configuration guidance, and the
//! actions that explain or recover a failed graph explanation without
//! leaving the memory detail.

use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use argos_osint_core::graph_explanation::{record_detail_lines, ExplainReport};
use argos_osint_core::provider_diag::ProviderFailure;
use argos_osint_core::store::ExplanationRecord;

use super::app::{App, ButtonId, Target};
use super::theme;

/// A failed graph explanation for the open memory.
#[derive(Clone, Debug, PartialEq)]
pub struct SummaryFailure {
    pub memory_id: String,
    pub job_id: String,
    pub event_id: String,
    pub reason: String,
    pub guidance: Option<String>,
    pub needs_config: bool,
    pub details: Vec<String>,
    /// Logging could not be written; disclosed on the card.
    pub logging_error: Option<String>,
    /// Shown from the saved diagnostic (reopened inside the retry cooldown).
    pub from_record: bool,
}

impl SummaryFailure {
    pub fn from_report(report: &ExplainReport, failure: &ProviderFailure) -> Self {
        let mut details = failure.detail_lines();
        details.push(format!(
            "Requests: {} of {} · admission wait {} ms (not counted)",
            report.attempts.len(),
            argos_osint_core::summarization::MAX_REQUESTS,
            report.admission_wait_ms
        ));
        details.extend(report.attempts.iter().map(|a| a.line()));
        if let Some(reason) = &report.fallback_reason {
            details.push(format!("Fallback: {reason}"));
        }
        if !report.job_id.is_empty() {
            details.push(format!("Job: {}", report.job_id));
        }
        details.push(format!("Request: {}", report.request_id));
        Self {
            memory_id: report.memory_id.clone(),
            job_id: report.job_id.clone(),
            event_id: report.event_id.clone(),
            reason: failure.summary(),
            guidance: failure.guidance(),
            needs_config: failure.needs_configuration(),
            details,
            logging_error: report.logging_error.clone(),
            from_record: false,
        }
    }

    /// A failure detected before any job started (e.g. no provider configured).
    pub fn local(memory_id: &str, failure: &ProviderFailure) -> Self {
        Self {
            memory_id: memory_id.into(),
            job_id: String::new(),
            event_id: String::new(),
            reason: failure.summary(),
            guidance: failure.guidance(),
            needs_config: failure.needs_configuration(),
            details: failure.detail_lines(),
            logging_error: None,
            from_record: false,
        }
    }

    pub fn from_record(rec: &ExplanationRecord) -> Self {
        Self {
            memory_id: rec.memory_id.clone(),
            job_id: rec.job_id.clone(),
            event_id: rec.event_id.clone(),
            reason: rec.reason.clone(),
            guidance: (!rec.guidance.is_empty()).then(|| rec.guidance.clone()),
            needs_config: rec.needs_config,
            details: record_detail_lines(rec),
            logging_error: None,
            from_record: true,
        }
    }
}

/// Card actions in paint/focus order. Logs and Job need a durable job;
/// Open Models appears only for credential/model/configuration failures.
pub fn actions(failure: &SummaryFailure) -> Vec<ButtonId> {
    let mut out = vec![ButtonId::SummaryDetails];
    if !failure.job_id.is_empty() {
        out.push(ButtonId::SummaryLogs);
        out.push(ButtonId::SummaryJob);
    }
    out.push(ButtonId::SummaryRetry);
    if failure.needs_config {
        out.push(ButtonId::SummaryModels);
    }
    out
}

pub fn is_card_button(button: ButtonId) -> bool {
    matches!(
        button,
        ButtonId::SummaryDetails
            | ButtonId::SummaryLogs
            | ButtonId::SummaryJob
            | ButtonId::SummaryRetry
            | ButtonId::SummaryModels
    )
}

fn label(button: ButtonId, details_open: bool, short: bool) -> &'static str {
    match (button, short) {
        (ButtonId::SummaryDetails, false) if details_open => "Hide details",
        (ButtonId::SummaryDetails, true) if details_open => "Hide",
        (ButtonId::SummaryDetails, false) => "View details",
        (ButtonId::SummaryDetails, true) => "Details",
        (ButtonId::SummaryLogs, false) => "View logs",
        (ButtonId::SummaryLogs, true) => "Logs",
        (ButtonId::SummaryJob, false) => "View job",
        (ButtonId::SummaryJob, true) => "Job",
        (ButtonId::SummaryRetry, false) => "Retry summary",
        (ButtonId::SummaryRetry, true) => "Retry",
        (ButtonId::SummaryModels, false) => "Open Models",
        (ButtonId::SummaryModels, true) => "Models",
        _ => "",
    }
}

/// Text rows of the card (before the buttons) at `width`.
fn text_rows(failure: &SummaryFailure, width: usize) -> Vec<(String, Style)> {
    let mut rows = Vec::new();
    let head = format!("⚠ AI summary failed · {}", failure.reason);
    for line in wrap(&head, width).into_iter().take(3) {
        rows.push((line, theme::error()));
    }
    if let Some(guidance) = &failure.guidance {
        for line in wrap(guidance, width).into_iter().take(3) {
            rows.push((line, theme::warn()));
        }
    }
    if let Some(err) = &failure.logging_error {
        for line in wrap(&format!("Logs could not be written: {err}"), width)
            .into_iter()
            .take(2)
        {
            rows.push((line, theme::warn()));
        }
    }
    rows
}

/// Button cells laid out left to right, wrapping to new rows. Long labels
/// are used when every button fits on one row.
pub fn button_cells(
    failure: &SummaryFailure,
    details_open: bool,
    area: Rect,
) -> Vec<(ButtonId, &'static str, Rect)> {
    let ids = actions(failure);
    let width = area.width as usize;
    // Rows needed when laid out greedily with long or short labels.
    let rows = |short: bool| -> usize {
        let (mut rows, mut used) = (1usize, 0usize);
        for id in &ids {
            let w = label(*id, details_open, short).chars().count() + 4;
            if used > 0 && used + 1 + w > width {
                rows += 1;
                used = 0;
            }
            used += if used > 0 { w + 1 } else { w };
        }
        rows
    };
    // Full labels unless they would need more than two rows.
    let short = rows(false) > 2;
    let mut out = Vec::new();
    let (mut x, mut y) = (area.x, area.y);
    for id in ids {
        let text = label(id, details_open, short);
        let w = (text.chars().count() + 4) as u16;
        if x > area.x && x + w > area.x + area.width {
            x = area.x;
            y += 1;
        }
        out.push((
            id,
            text,
            Rect {
                x,
                y,
                width: w.min(area.width),
                height: 1,
            },
        ));
        x = x.saturating_add(w + 1);
    }
    out
}

/// Card rows, including the button rows and one blank separator.
pub fn height(failure: &SummaryFailure, details_open: bool, inner: Rect) -> u16 {
    let text = text_rows(failure, inner.width as usize).len() as u16;
    let probe = Rect {
        y: 0,
        height: u16::MAX,
        ..inner
    };
    let rows = button_cells(failure, details_open, probe)
        .last()
        .map(|(_, _, r)| r.y + 1)
        .unwrap_or(0);
    (text + rows + 1).min(inner.height)
}

fn button_row_area(failure: &SummaryFailure, inner: Rect) -> Rect {
    let text = text_rows(failure, inner.width as usize).len() as u16;
    Rect {
        y: inner.y + text,
        height: inner.height.saturating_sub(text),
        ..inner
    }
}

/// Draw the card at the top of `inner` (the summary panel's inner area).
pub fn draw(frame: &mut Frame, app: &App, failure: &SummaryFailure, inner: Rect) {
    let width = inner.width as usize;
    let rows = text_rows(failure, width);
    let lines: Vec<Line> = rows
        .iter()
        .map(|(text, style)| Line::from(Span::styled(text.clone(), *style)))
        .collect();
    let text_h = (lines.len() as u16).min(inner.height);
    frame.render_widget(
        Paragraph::new(lines),
        Rect {
            height: text_h,
            ..inner
        },
    );
    let buttons = button_row_area(failure, inner);
    for (id, text, rect) in button_cells(failure, app.summary_details_open, buttons) {
        if rect.y >= inner.y + inner.height {
            break;
        }
        app.layout.borrow_mut().register(Target::Button(id), rect);
        let focused = app.focus == Target::Button(id);
        let style = if focused {
            theme::selected()
        } else {
            theme::accent()
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(format!("[ {text} ]"), style))),
            rect,
        );
    }
}

/// Which card button is under (x, y), if any.
pub fn button_at(
    app: &App,
    failure: &SummaryFailure,
    inner: Rect,
    x: u16,
    y: u16,
) -> Option<ButtonId> {
    let buttons = button_row_area(failure, inner);
    button_cells(failure, app.summary_details_open, buttons)
        .into_iter()
        .find(|(_, _, r)| super::ui::contains(*r, x, y))
        .map(|(id, _, _)| id)
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut out = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let need = line.chars().count() + usize::from(!line.is_empty()) + word.chars().count();
        if need > width && !line.is_empty() {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
        while line.chars().count() > width {
            let head: String = line.chars().take(width).collect();
            let tail: String = line.chars().skip(width).collect();
            out.push(head);
            line = tail;
        }
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}
