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

/// Bounds a usize width so a render never panics on a narrow viewport.
pub fn bounded(width: usize, min: usize, max: usize) -> usize {
    width.clamp(min, max.max(min))
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
        for (index, (_, value)) in series.iter().enumerate() {
            if *value == 0 || denominator == 0 {
                continue;
            }
            let fractional = Some(index) != final_index;
            segments.push((
                theme::series_style(index),
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
/// truncates from the left. Column `i` uses `VBLOCKS` scaled to `height`.
/// Returns `height` `Line`s, top row first.
pub fn trend_columns(values: &[u64], width: usize, height: u16) -> Vec<Line<'static>> {
    let rows = height as usize;
    if rows == 0 || width == 0 {
        return Vec::new();
    }
    // Newest bucket last, so the chart reads left to right in time: a short
    // series is padded on the LEFT (its oldest buckets are not on screen yet) and
    // a long one drops its oldest buckets off the left edge.
    let visible: Vec<Option<u64>> = if values.len() >= width {
        (0..width)
            .map(|column| values.get(values.len() - width + column).copied())
            .collect()
    } else {
        let pad = width - values.len();
        (0..width)
            .map(|column| {
                if column < pad {
                    None
                } else {
                    values.get(column - pad).copied()
                }
            })
            .collect()
    };
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

/// Cells taken by one character: control characters take none, wide characters
/// take two, everything else one.
fn cell_width(ch: char) -> usize {
    if ch.is_control() {
        return 0;
    }
    let code = ch as u32;
    let wide = (0x1100..=0x115F).contains(&code)
        || (0x2E80..=0xA4CF).contains(&code)
        || (0xAC00..=0xD7A3).contains(&code)
        || (0xF900..=0xFAFF).contains(&code)
        || (0xFE10..=0xFE19).contains(&code)
        || (0xFE30..=0xFE6F).contains(&code)
        || (0xFF00..=0xFF60).contains(&code)
        || (0xFFE0..=0xFFE6).contains(&code)
        || (0x1F300..=0x1FAFF).contains(&code);
    if wide {
        2
    } else {
        1
    }
}

/// Display cells taken by `text`.
fn text_width(text: &str) -> usize {
    text.chars().map(cell_width).sum()
}

/// `text` at exactly `width` cells: padded on the right when it fits, cut with
/// `…` when it does not, so a label can never push a chart sideways.
fn fit_label(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if text_width(text) <= width {
        let mut out = text.to_string();
        out.push_str(&" ".repeat(width - text_width(text)));
        return out;
    }
    let keep = width - 1;
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = cell_width(ch);
        if used + w > keep {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(short.spans[1].style.fg, Some(theme::series(0)));
        assert_eq!(short.spans[2].style.fg, Some(theme::series(1)));
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
        assert_eq!(line.spans[3].style.fg, Some(theme::series(1)));
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
    fn trend_columns_truncates_a_long_series_from_the_left() {
        // Newest bucket last: the first three buckets drop off the left edge.
        let columns = trend_columns(&[1, 2, 3, 4, 5, 6], 3, 2);
        assert_eq!(text(&columns[0]), "▂▅█");
        assert_eq!(text(&columns[1]), "███");
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
    fn bounded_clamps_widths_for_a_narrow_viewport() {
        assert_eq!(bounded(100, 20, 240), 100);
        assert_eq!(bounded(5, 20, 240), 20);
        assert_eq!(bounded(400, 20, 240), 240);
        assert_eq!(bounded(0, 0, 0), 0);
        // A reversed bound still leaves a usable width instead of panicking.
        assert_eq!(bounded(5, 240, 20), 240);
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
