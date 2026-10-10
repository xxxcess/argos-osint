//! Hand-drawn charts for the Profile dashboard.
//!
//! Every chart here is a `Paragraph` line list built from block glyphs. The
//! `ratatui::Chart` and `ratatui::BarChart` widgets are deliberately unused:
//! the dashboard needs stacked, ranked, metered and trend shapes on a fixed
//! cell grid, the fill of a segment has to carry the exact series colour from
//! [`super::theme::SERIES`], and a narrow viewport must never panic. One shared
//! renderer draws them all, so a widget cannot smuggle in its own geometry — the
//! same precedent as `tui/atlas_table.rs`. Connectors are orthogonal, nothing
//! is rounded or shadowed, and every glyph occupies exactly one cell, so a
//! line never wraps oddly.
//!
//! A missing metric renders `N/A`, never `0`: a value the snapshot never
//! collected must not read as a measured zero.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use super::theme;

/// Stable series identity, including semantic outcome colours.
pub fn series_style(key: &str) -> Style {
    let color = match key {
        "completed" | "completed_nonempty" | "completed_with_evidence" => theme::GREEN,
        "failed" => theme::RED,
        "partial" | "blocked" => theme::WARN,
        "cancelled" | "unknown" => theme::MUTED,
        _ => theme::series(key.bytes().fold(0usize, |hash, byte| {
            hash.wrapping_mul(31).wrapping_add(byte as usize)
        })),
    };
    Style::default().fg(color).bg(theme::BG)
}

#[derive(Clone, Debug)]
pub struct TimeBucket {
    pub start: String,
    pub end: String,
    pub series: Vec<(String, u64)>,
    /// False before collection/retention coverage. It is not a measured zero.
    pub available: bool,
}

#[derive(Clone, Debug)]
pub struct SeriesPoint {
    pub start: String,
    pub end: String,
    pub series: Vec<(String, Option<f64>)>,
    pub n: u64,
}

/// Duration/ratio points page around the selection. No percentile is averaged
/// and no missing observation is interpolated.
pub fn series_plot(
    points: &[SeriesPoint],
    width: usize,
    height: usize,
    selected: usize,
) -> Vec<Line<'static>> {
    if width < 14 || height < 5 || points.is_empty() {
        return Vec::new();
    }
    let capacity = ((width - 7) / 3).max(1);
    let start = selected.min(points.len() - 1) / capacity * capacity;
    let page = &points[start..(start + capacity).min(points.len())];
    let peak = page
        .iter()
        .flat_map(|point| point.series.iter().filter_map(|(_, value)| *value))
        .filter(|value| value.is_finite())
        .fold(0.0, f64::max)
        .max(1.0);
    let scale = nice_scale(peak.ceil().min(u64::MAX as f64) as u64) as f64;
    let rows = height - 3;
    let mut lines = Vec::new();
    for row in (0..rows).rev() {
        let mut spans = vec![Span::styled(
            if row == rows - 1 {
                format!("{:>5.0} │", scale)
            } else {
                "      │".into()
            },
            theme::dim(),
        )];
        for (index, point) in page.iter().enumerate() {
            let mark = point.series.iter().find(|(_, value)| {
                value.is_some_and(|value| {
                    value.is_finite()
                        && ((value / scale * (rows - 1) as f64).round() as usize) == row
                })
            });
            spans.push(match mark {
                Some((key, _)) => Span::styled(
                    "●  ",
                    if start + index == selected {
                        series_style(key).bg(theme::SELECT)
                    } else {
                        series_style(key)
                    },
                ),
                None => Span::raw("   "),
            });
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::from(Span::styled(
        format!("    0 └{}", "─".repeat(width - 7)),
        theme::dim(),
    )));
    lines.push(Line::raw(super::components::clip_text(
        &format!(
            "Points {}–{} / {}",
            start + 1,
            start + page.len(),
            points.len()
        ),
        width,
    )));
    let stamp = page[0].start.get(5..).unwrap_or(&page[0].start);
    lines.push(Line::raw(super::components::clip_text(stamp, width)));
    lines
}

impl TimeBucket {
    pub fn total(&self) -> u64 {
        self.series.iter().map(|(_, value)| *value).sum()
    }
}

/// Count buckets coarsen by sums, retaining both ends of the complete window.
pub fn coarsen_counts(buckets: &[TimeBucket], capacity: usize) -> Vec<TimeBucket> {
    let group = buckets.len().div_ceil(capacity.max(1)).max(1);
    buckets
        .chunks(group)
        .map(|chunk| {
            let mut series = std::collections::BTreeMap::<String, u64>::new();
            for bucket in chunk {
                for (key, value) in &bucket.series {
                    *series.entry(key.clone()).or_default() += value;
                }
            }
            TimeBucket {
                start: chunk[0].start.clone(),
                end: chunk[chunk.len() - 1].end.clone(),
                series: series.into_iter().collect(),
                available: chunk.iter().all(|bucket| bucket.available),
            }
        })
        .collect()
}

fn nice_scale(peak: u64) -> u64 {
    let mut base = 1u64;
    while base.saturating_mul(10) < peak && base <= u64::MAX / 10 {
        base *= 10;
    }
    [1u64, 2, 5, 10]
        .into_iter()
        .map(|step| base.saturating_mul(step))
        .find(|scale| *scale >= peak)
        .unwrap_or(u64::MAX)
}

/// Measured TimePlot. Selection refers to original buckets, never an averaged point.
pub fn time_plot(
    buckets: &[TimeBucket],
    width: usize,
    height: usize,
    selected: usize,
) -> Vec<Line<'static>> {
    if buckets.is_empty() {
        return vec![Line::raw("Empty window")];
    }
    if width < 14 || height < 4 {
        return Vec::new();
    }
    let axis = 7;
    let capacity = (width - axis) / 3;
    let visible = coarsen_counts(buckets, capacity);
    let group = buckets.len().div_ceil(capacity.max(1)).max(1);
    let peak = nice_scale(
        visible
            .iter()
            .map(TimeBucket::total)
            .max()
            .unwrap_or(0)
            .max(1),
    );
    let rows = height - 4;
    let mut out = Vec::new();
    let mut totals = vec![Span::raw(" ".repeat(axis))];
    for bucket in &visible {
        let value = if bucket.available {
            bucket.total().to_string()
        } else {
            " · ".into()
        };
        totals.push(Span::styled(
            if value.len() <= 3 {
                format!("{value:<3}")
            } else {
                "   ".into()
            },
            theme::dim(),
        ));
    }
    out.push(Line::from(totals));
    for row in (0..rows).rev() {
        let label = if row == rows - 1 {
            format!("{peak:>5} │")
        } else {
            "      │".into()
        };
        let mut spans = vec![Span::styled(label, theme::dim())];
        for (index, bucket) in visible.iter().enumerate() {
            let amount = if bucket.available {
                (bucket.total() as u128 * rows as u128 * 8 / peak as u128) as usize
            } else {
                0
            };
            let fill = amount.saturating_sub(row * 8).min(8);
            let glyph = if fill == 0 { " " } else { VBLOCKS[fill - 1] };
            let position =
                ((row * 8 + fill / 2) as u128 * peak as u128 / (rows * 8) as u128) as u64;
            let mut sum = 0;
            let key = bucket
                .series
                .iter()
                .find_map(|(key, value)| {
                    sum += value;
                    (sum > position).then_some(key.as_str())
                })
                .unwrap_or("total");
            let style = if index == selected / group {
                series_style(key).bg(theme::SELECT)
            } else {
                series_style(key)
            };
            spans.push(Span::styled(format!("{glyph}{glyph} "), style));
        }
        out.push(Line::from(spans));
    }
    out.push(Line::from(Span::styled(
        format!("    0 └{}", "─".repeat(width - axis)),
        theme::dim(),
    )));
    let mut labels = vec![Span::raw(" ".repeat(axis))];
    for index in 0..visible.len() {
        labels.push(Span::styled(
            if index == selected / group {
                "▲  "
            } else {
                "   "
            },
            theme::accent(),
        ));
    }
    out.push(Line::from(labels));
    let short = |stamp: &str| {
        if stamp.len() >= 13 {
            format!("{} {}", &stamp[5..10], &stamp[11..13])
        } else if stamp.len() >= 10 {
            stamp[5..10].to_owned()
        } else {
            stamp.to_owned()
        }
    };
    let first = short(&buckets[0].start);
    let last = short(&buckets[buckets.len() - 1].end);
    let remaining = width.saturating_sub(axis);
    let label = if first.len() + last.len() < remaining {
        let gap = (visible.len() * 3)
            .min(remaining)
            .saturating_sub(first.len() + last.len())
            .max(1);
        format!("{first}{}{last}", " ".repeat(gap))
    } else if first.len() <= remaining {
        first
    } else {
        String::new()
    };
    out.push(Line::from(Span::styled(
        format!("{}{label}", " ".repeat(axis)),
        theme::dim(),
    )));
    out
}

/// Full- and part-width block glyphs, coarse to fine. `BLOCKS[0]` fills a whole
/// cell and `BLOCKS[7]` a single eighth of one.
const BLOCKS: [&str; 8] = ["█", "▉", "▊", "▋", "▌", "▍", "▎", "▏"];

/// Vertical blocks, low to high: `VBLOCKS[k]` fills `k + 1` eighths of a cell.
const VBLOCKS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// Mid-dot that keeps the unfilled part of a bar track visible.
const TRACK: &str = "·";

/// "N/A" for a missing value, never `0`: a metric that is unavailable must not
/// read as zero.
pub fn unavailable() -> &'static str {
    "N/A"
}

/// Formats a count with thousands separators.
pub fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Formats an optional percentage. `None` renders "N/A".
pub fn percent(value: Option<f64>) -> String {
    match value {
        Some(value) if value.is_finite() => format!("{value:.1}%"),
        _ => unavailable().to_string(),
    }
}

/// Formats an optional duration in milliseconds as `1.2s`/`450ms`/`2m 05s`.
/// `None`, and any negative input, renders "N/A".
pub fn duration_ms(value: Option<i64>) -> String {
    let ms = match value {
        Some(ms) if ms >= 0 => ms,
        _ => return unavailable().to_string(),
    };
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let whole_seconds = ms / 1_000;
    if whole_seconds < 60 {
        return format!("{:.1}s", ms as f64 / 1_000.0);
    }
    format!("{}m {:02}s", whole_seconds / 60, whole_seconds % 60)
}

/// Formats a sample size, e.g. "n=128".
/// `n=` plus the denominator, with thousands separators.
pub fn sample_size(n: u64) -> String {
    format!("n={}", count(n))
}

/// The single shared stacked-bar renderer.
///
/// Renders one row: `label` padded to `label_width`, a space, then a bar of at
/// most `bar_cells` cells whose segments are proportional to `series` over
/// `total` (when `total` is 0 or `total < sum(series)`, fall back to the sum of
/// the series as the denominator). Each segment is `BLOCKS[0]` repeated, with a
/// single fractional `BLOCKS[k]` at the trailing edge of a non-final segment
/// when the proportional width has a remainder. The label is `theme::dim()`,
/// segment `i` is `theme::series(i)`.
///
/// Unfilled cells are a `theme::dim()` mid-dot `·` so the track stays visible.
/// The line is at most `label_width + 1 + bar_cells` cells wide.
pub fn stacked_bar(
    label: &str,
    label_width: usize,
    series: &[(String, u64)],
    bar_cells: usize,
    total: u64,
) -> Line<'static> {
    let mut spans = vec![Span::styled(fit_label(label, label_width), theme::dim())];
    if label_width > 0 {
        spans.push(Span::styled(" ".to_string(), theme::dim()));
    }
    if bar_cells > 0 {
        let sum: u64 = series.iter().map(|(_, value)| *value).sum();
        // The caller's total only bounds the bar when it covers the series; a
        // partial breakdown falls back to the series sum, so the bar still
        // fills and the ratio between segments stays true.
        let denominator = if total > 0 && total >= sum {
            total
        } else {
            sum
        };
        // The trailing segment leaves its fraction behind: the end of the bar
        // stays flush with the track even when every fraction is dropped.
        let final_index = series.iter().rposition(|(_, value)| *value > 0);
        let mut segments: Vec<(Style, u64)> = Vec::with_capacity(series.len());
        for (index, (key, value)) in series.iter().enumerate() {
            if *value == 0 || denominator == 0 {
                continue;
            }
            let fractional = Some(index) != final_index;
            segments.push((
                series_style(key),
                eighths(*value, denominator, bar_cells, fractional),
            ));
        }
        spans.extend(render_segments(&segments, bar_cells));
    }
    Line::from(spans)
}

/// A ranked horizontal bar for table columns: `value/max` of `width` cells.
/// `max == 0` or `value <= 0` renders an empty track.
pub fn rank_bar(value: u64, max: u64, width: usize) -> Line<'static> {
    let segments = if max == 0 || value == 0 {
        Vec::new()
    } else {
        vec![(theme::series_style(0), eighths(value, max, width, true))]
    };
    Line::from(render_segments(&segments, width))
}

/// A small vertical column block for a trend series (one column per bucket,
/// newest last). `height` rows, `width` columns; `width` larger than
/// `values.len()` is padded on the left with blank columns, `width` smaller
/// coarsens count buckets by summing. Column `i` uses `VBLOCKS` scaled to `height`.
/// Returns `height` `Line`s, top row first.
pub fn trend_columns(values: &[u64], width: usize, height: u16) -> Vec<Line<'static>> {
    let rows = height as usize;
    if rows == 0 || width == 0 {
        return Vec::new();
    }
    let group = values.len().div_ceil(width).max(1);
    let coarsened: Vec<u64> = values
        .chunks(group)
        .map(|chunk| chunk.iter().sum())
        .collect();
    let pad = width.saturating_sub(coarsened.len());
    let visible: Vec<Option<u64>> = (0..width)
        .map(|column| {
            column
                .checked_sub(pad)
                .and_then(|index| coarsened.get(index).copied())
        })
        .collect();
    let peak = visible.iter().flatten().copied().max().unwrap_or(0);
    let mut lines: Vec<Line<'static>> = Vec::with_capacity(rows);
    for (index, row) in (0..rows).rev().enumerate() {
        let _ = index;
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(width);
        for bucket in &visible {
            let (glyph, style) = match *bucket {
                Some(value) => column_cell(value, peak, rows, row),
                None => (" ", theme::dim()),
            };
            spans.push(Span::styled(glyph.to_string(), style));
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// A single-row horizontal meter for a ratio. `None` renders the word `N/A`
/// across the track; the track is `·` with `█` fill.
pub fn meter(ratio: Option<f64>, width: usize) -> Line<'static> {
    let ratio = ratio.filter(|ratio| ratio.is_finite());
    let segments = match ratio {
        Some(ratio) => vec![(theme::series_style(0), eighths_of_ratio(ratio, width))],
        None => return Line::from(Span::styled(fit_label(unavailable(), width), theme::dim())),
    };
    Line::from(render_segments(&segments, width))
}

// ---------------------------------------------------------------------------
// Shared geometry
// ---------------------------------------------------------------------------

/// The one renderer every horizontal chart in this module draws through:
/// `segments` are `(style, eighths)` widths left to right and `cells` is the
/// track width. A segment draws whole cells plus, when it holds a fraction, a
/// single part block at its trailing edge; whatever it leaves behind is a
/// `theme::dim()` `·` track. Widths are clamped here, so no caller can push a
/// bar past its viewport.
fn render_segments(segments: &[(Style, u64)], cells: usize) -> Vec<Span<'static>> {
    let mut spans: Vec<Span<'static>> = Vec::with_capacity(segments.len() + 1);
    let mut used = 0usize;
    for &(style, filled) in segments {
        if filled == 0 || used >= cells {
            continue;
        }
        let whole = (filled / 8) as usize;
        let whole = whole.min(cells - used);
        if whole > 0 {
            spans.push(Span::styled(BLOCKS[0].repeat(whole), style));
            used += whole;
        }
        let remainder = (filled % 8) as usize;
        if remainder > 0 && used < cells {
            spans.push(Span::styled(String::from(BLOCKS[8 - remainder]), style));
            used += 1;
        }
    }
    if used < cells {
        spans.push(Span::styled(TRACK.repeat(cells - used), theme::dim()));
    }
    spans
}

/// `value` of `denominator` across `cells` cells, in eighths of a cell. The
/// fractional eighth is dropped when `fractional` is false, so the trailing
/// edge of a final segment never overhangs the track.
fn eighths(value: u64, denominator: u64, cells: usize, fractional: bool) -> u64 {
    if value == 0 || denominator == 0 || cells == 0 {
        return 0;
    }
    let exact = scaled_eighths(value, denominator, cells);
    if fractional {
        exact
    } else {
        exact - exact % 8
    }
}

/// `value` of `denominator` across `cells` cells, exact eighths.
fn scaled_eighths(value: u64, denominator: u64, cells: usize) -> u64 {
    let cap = cap_eighths(cells);
    (value as u128 * cap as u128 / denominator as u128).min(cap as u128) as u64
}

/// A clamped 0..=1 ratio across `cells` cells, in eighths of a cell.
fn eighths_of_ratio(ratio: f64, cells: usize) -> u64 {
    let cap = cap_eighths(cells);
    ((ratio.clamp(0.0, 1.0) * cap as f64).round() as u64).min(cap)
}

/// Eight times `cells`, the largest legible width a track can scale to.
fn cap_eighths(cells: usize) -> u64 {
    (cells as u64).saturating_mul(8)
}

/// One cell of a trend column. `value` is the bucket, `peak` the tallest bucket
/// on screen, and `row` counts up from the floor of the chart, so the caller
/// renders the rows bottom-up.
fn column_cell(value: u64, peak: u64, rows: usize, row: usize) -> (&'static str, Style) {
    if value == 0 || peak == 0 || rows == 0 {
        return (" ", theme::dim());
    }
    let style = theme::series_style(0);
    // Scale against the tallest column, so the tallest reaches the top row.
    let scaled = scaled_eighths(value, peak, rows);
    let fill = (scaled / 8) as usize;
    let remainder = (scaled % 8) as usize;
    if row < fill {
        (VBLOCKS[7], style)
    } else if row == fill && remainder > 0 {
        // `VBLOCKS` runs low to high, so k eighths is the (k-1) block.
        (VBLOCKS[remainder - 1], style)
    } else {
        (" ", theme::dim())
    }
}

// ---------------------------------------------------------------------------
// Single-cell text helpers
// ---------------------------------------------------------------------------

fn text_width(text: &str) -> usize {
    super::components::text_width(text)
}
fn fit_label(text: &str, width: usize) -> String {
    let text = super::components::clip_text(text, width);
    format!(
        "{text}{}",
        " ".repeat(width.saturating_sub(text_width(&text)))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_coarsening_preserves_totals_boundaries_and_unknown_history() {
        let buckets: Vec<_> = (0..101)
            .map(|i| TimeBucket {
                start: format!("start-{i}"),
                end: format!("end-{i}"),
                series: vec![("completed".into(), i)],
                available: i > 0,
            })
            .collect();
        let coarse = coarsen_counts(&buckets, 7);
        assert!(coarse.len() <= 7);
        assert_eq!(coarse.iter().map(TimeBucket::total).sum::<u64>(), 5050);
        assert_eq!(coarse.first().unwrap().start, "start-0");
        assert_eq!(coarse.last().unwrap().end, "end-100");
        assert!(!coarse[0].available);
    }

    #[test]
    fn time_plot_fits_actual_rectangle() {
        let buckets: Vec<_> = (0..101)
            .map(|i| TimeBucket {
                start: format!("2026-10-10T{i:02}"),
                end: "2026-10-11T00:00:00Z".into(),
                series: vec![("completed".into(), i)],
                available: true,
            })
            .collect();
        for width in [14, 20, 40, 80, 160] {
            for height in [5, 8, 12] {
                let lines = time_plot(&buckets, width, height, 100);
                assert!(lines.len() <= height, "height {height}: {}", lines.len());
                assert!(
                    lines.iter().all(|line| line.width() <= width),
                    "width {width}: {lines:?}"
                );
            }
        }
    }

    #[test]
    fn semantic_series_colours_survive_filter_reordering() {
        let a = stacked_bar(
            "",
            0,
            &[("failed".into(), 1), ("completed".into(), 1)],
            8,
            2,
        );
        let b = stacked_bar(
            "",
            0,
            &[("completed".into(), 1), ("failed".into(), 1)],
            8,
            2,
        );
        assert_eq!(a.spans[1].style.fg, b.spans[2].style.fg);
        assert_eq!(a.spans[1].style.fg, Some(theme::RED));
    }

    fn text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .concat()
    }

    fn cells(line: &Line<'_>) -> usize {
        line.spans
            .iter()
            .map(|span| text_width(&span.content))
            .sum()
    }

    fn segment(name: &str, value: u64) -> (String, u64) {
        (name.to_string(), value)
    }

    #[test]
    fn stacked_bar_uses_series_sum_when_total_is_zero() {
        // total == 0 falls back to the series sum, and a total below the sum
        // does too, so a partial breakdown still fills the track.
        let exact = stacked_bar("l", 0, &[segment("a", 3), segment("b", 1)], 8, 0);
        assert_eq!(text(&exact), "████████");
        assert_eq!(cells(&exact), 8);
        let short = stacked_bar("l", 0, &[segment("a", 2), segment("b", 2)], 8, 3);
        assert_eq!(text(&short), "████████");
        assert_eq!(short.spans[1].style.fg, series_style("a").fg);
        assert_eq!(short.spans[2].style.fg, series_style("b").fg);
    }

    #[test]
    fn stacked_bar_renders_an_empty_track_for_all_zero_series() {
        let line = stacked_bar("l", 0, &[segment("a", 0), segment("b", 0)], 8, 0);
        assert_eq!(text(&line), "········");
        assert_eq!(cells(&line), 8);
    }

    #[test]
    fn stacked_bar_segments_stay_within_bar_cells() {
        let series = [
            segment("a", 1),
            segment("b", 7),
            segment("c", 640),
            segment("d", 3),
        ];
        for width in [0_usize, 1, 3, 13, 40] {
            for total in [0_u64, 651, 100_000] {
                let line = stacked_bar("l", 0, &series, width, total);
                assert!(cells(&line) <= width, "overflow at {width}c, total {total}");
                assert_eq!(cells(&line), width);
            }
        }
    }

    #[test]
    fn stacked_bar_rounds_the_trailing_edge_of_a_non_final_segment() {
        // 1/3 of 10 cells is 3 cells + 2 eighths, so the first segment ends in
        // one part block; the final segment drops its own fraction.
        let line = stacked_bar("l", 0, &[segment("a", 1), segment("b", 2)], 10, 3);
        assert_eq!(line.spans.len(), 4); // label, segment, part block, segment
        assert_eq!(line.spans[0].content, "");
        assert_eq!(line.spans[1].content, "███");
        assert_eq!(line.spans[2].content, "▎");
        assert_eq!(line.spans[1].style.fg, line.spans[2].style.fg);
        assert_eq!(line.spans[3].content, "██████");
        assert_eq!(line.spans[3].style.fg, series_style("b").fg);
        assert_eq!(text(&line), "███▎██████");
        assert_eq!(cells(&line), 10);
    }

    #[test]
    fn stacked_bar_keeps_a_dim_track_for_unfilled_cells() {
        let line = stacked_bar("l", 0, &[segment("a", 1)], 10, 10);
        assert_eq!(text(&line), "█·········");
        assert_eq!(cells(&line), 10);
        let track = line.spans.last().expect("track span");
        assert_eq!(track.style.fg, Some(theme::DIM));
    }

    #[test]
    fn stacked_bar_truncates_a_long_label_with_an_ellipsis() {
        let line = stacked_bar("engineering", 6, &[segment("a", 1), segment("b", 1)], 4, 2);
        assert_eq!(line.spans[0].content, "engin…");
        assert_eq!(line.spans[0].style.fg, Some(theme::DIM));
        assert_eq!(cells(&line), 6 + 1 + 4);
        let short = stacked_bar("ab", 6, &[], 4, 0);
        assert_eq!(short.spans[0].content, "ab    ");
    }

    #[test]
    fn stacked_bar_handles_zero_bar_cells_and_empty_series() {
        let none = stacked_bar("x", 4, &[], 0, 0);
        assert_eq!(cells(&none), 5);
        let empty = stacked_bar("x", 4, &[], 6, 0);
        assert_eq!(text(&empty), "x    ······");
        let no_label = stacked_bar("x", 0, &[segment("a", 1)], 4, 4);
        assert_eq!(text(&no_label), "█···");
        assert_eq!(cells(&no_label), 4);
    }

    #[test]
    fn trend_columns_returns_height_lines_of_width_cells() {
        let columns = trend_columns(&[1, 2, 3, 4, 5, 6], 6, 4);
        assert_eq!(columns.len(), 4);
        for line in &columns {
            assert_eq!(cells(line), 6);
            assert_eq!(line.spans.len(), 6);
        }
        // The tallest bucket reaches the top row of the chart.
        assert!(text(&columns[0]).contains(VBLOCKS[7]));
    }

    #[test]
    fn trend_columns_pads_a_short_series_on_the_left() {
        let columns = trend_columns(&[4], 3, 2);
        assert_eq!(columns.len(), 2);
        assert_eq!(text(&columns[0]), "  █");
        assert_eq!(text(&columns[1]), "  █");
    }

    #[test]
    fn trend_columns_sums_a_long_count_series_without_dropping_history() {
        // All six counts survive as paired sums: 3, 7, 11.
        let columns = trend_columns(&[1, 2, 3, 4, 5, 6], 3, 2);
        assert_eq!(text(&columns[0]), " ▂█");
        assert_eq!(text(&columns[1]), "▄██");
    }

    #[test]
    fn trend_columns_scales_the_partial_top_cell() {
        // Two buckets across two columns, three rows tall. The peak fills every
        // row; the half-height bucket fills one whole cell and a quarter of the
        // next, so its column stops two rows short of the top.
        let columns = trend_columns(&[1, 2], 2, 3);
        assert_eq!(columns.len(), 3);
        assert_eq!(text(&columns[0]), " █");
        assert_eq!(text(&columns[1]), "▄█");
        // Two columns are two cells: the bottom row is full under both buckets.
        assert_eq!(text(&columns[2]), "██");
        assert_eq!(cells(&columns[2]), 2);
    }

    #[test]
    fn trend_columns_renders_nothing_without_a_height_or_a_width() {
        assert!(trend_columns(&[1, 2, 3], 4, 0).is_empty());
        assert!(trend_columns(&[1, 2, 3], 0, 4).is_empty());
        assert!(trend_columns(&[], 4, 3)
            .iter()
            .all(|line| text(line) == "    "));
    }

    #[test]
    fn rank_bar_handles_a_zero_maximum() {
        let line = rank_bar(0, 0, 6);
        assert_eq!(text(&line), "······");
        assert_eq!(cells(&line), 6);
        let zero_value = rank_bar(0, 12, 4);
        assert_eq!(text(&zero_value), "····");
    }

    #[test]
    fn rank_bar_scales_value_over_maximum() {
        let half = rank_bar(5, 10, 8);
        assert_eq!(text(&half), "████····");
        assert_eq!(cells(&half), 8);
        // A value past the maximum is clamped to the track, never past it.
        let over = rank_bar(40, 10, 6);
        assert_eq!(text(&over), "██████");
    }

    #[test]
    fn meter_renders_na_when_the_ratio_is_missing() {
        let line = meter(None, 10);
        assert!(text(&line).starts_with("N/A"));
        assert_eq!(cells(&line), 10);
        // A two-cell track cannot hold the word, so it is cut, not wrapped.
        assert_eq!(meter(None, 2).spans[0].content, "N…");
    }

    #[test]
    fn meter_fills_the_track_for_a_full_ratio() {
        assert_eq!(text(&meter(Some(1.0), 5)), "█████");
        assert_eq!(text(&meter(Some(0.5), 4)), "██··");
        assert_eq!(text(&meter(Some(0.0), 4)), "····");
        assert_eq!(text(&meter(Some(-1.0), 4)), "····");
        assert_eq!(text(&meter(Some(2.0), 4)), "████");
    }

    #[test]
    fn unavailable_metrics_render_na_not_zero() {
        assert_eq!(unavailable(), "N/A");
        assert_eq!(percent(None), "N/A");
        assert_eq!(duration_ms(None), "N/A");
        assert_eq!(duration_ms(Some(-1)), "N/A");
        assert_eq!(percent(Some(0.0)), "0.0%");
        assert_eq!(duration_ms(Some(0)), "0ms");
        let missing = meter(None, 6);
        assert!(!text(&missing).contains('0'));
    }

    #[test]
    fn count_formats_thousands_separators() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1_234), "1,234");
        assert_eq!(count(1_234_567), "1,234,567");
    }

    #[test]
    fn duration_ms_formats_milliseconds_seconds_and_minutes() {
        assert_eq!(duration_ms(Some(450)), "450ms");
        assert_eq!(duration_ms(Some(1_200)), "1.2s");
        assert_eq!(duration_ms(Some(2_500)), "2.5s");
        assert_eq!(duration_ms(Some(125_000)), "2m 05s");
        assert_eq!(duration_ms(Some(60_000)), "1m 00s");
    }

    #[test]
    fn percent_formats_optional_values() {
        assert_eq!(percent(Some(12.34)), "12.3%");
        assert_eq!(percent(Some(99.999)), "100.0%");
        assert_eq!(percent(Some(f64::NAN)), "N/A");
        assert_eq!(percent(None), "N/A");
    }

    #[test]
    fn sample_size_labels_the_denominator() {
        assert_eq!(sample_size(128), "n=128");
        assert_eq!(sample_size(0), "n=0");
        assert_eq!(sample_size(12_345), "n=12,345");
    }

    #[test]
    fn every_glyph_is_single_cell() {
        for glyph in BLOCKS.iter().chain(VBLOCKS.iter()) {
            assert_eq!(text_width(glyph), 1, "{glyph} must occupy one cell");
        }
        assert_eq!(text_width(TRACK), 1, "{TRACK} must occupy one cell");
        let line = stacked_bar("label", 6, &[segment("a", 3), segment("b", 1)], 12, 4);
        assert_eq!(cells(&line), 6 + 1 + 12);
        for column in trend_columns(&[1, 2, 3], 3, 4) {
            assert_eq!(cells(&column), 3);
        }
        for row in [rank_bar(3, 9, 8), meter(Some(0.25), 8)] {
            assert_eq!(cells(&row), 8);
        }
    }
}
