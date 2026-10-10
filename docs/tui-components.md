# TUI components

The [design contract](tui-design-spec.md) preserves the palette in `theme.rs`. Profile follows the [20-view dashboard contract](profile-analytics-dashboard.md) and retains metric semantics/history in [profile-dashboard-and-search.md](profile-dashboard-and-search.md). Components render immutable data; storage reads belong to background refresh/detail delivery.

## AnalyticsCard and Dashboard preset

`profile_layout::LayoutResult::new(area, page, count, offset)` is the pure shared geometry result for the **Dashboard** preset: tabs, section navigation, controls, KPI cards, visible analytic panels, scroll extent and status. Drawing, focus registration and pointer routing consume those same rectangles. `PanelRect` includes visible `rect`, virtual `height` and `source_offset`, preserving panel geometry while cropping scrolled rows. `profile_components::{kpi, panel, table_row, clipped}` composes reusable Profile KPI/chart/table panels; general text/editor/transcript primitives stay in `components`.

```rust,ignore
let layout = LayoutResult::new(area, DashboardPage::Summary, 4, page_offset);
for geometry in &layout.panels {
    profile_components::panel(frame, geometry, title, focused, lines);
}
```

The body has two columns at ≥120 cells and one otherwise, with a one-cell column/row gutter. KPIs use four columns ≥120, two at 80–119 and compact two-column values below 80. Summary pairs 18-row panels; Recon pairs 11-row panels with a full-width 13-row unresolved table. Other page arrangements follow the dashboard matrix. Chart minimum height is 11, table minimum 9; reduce chrome and scroll instead of squeezing. Below 60×18 retain navigation and a size notice.

**AnalyticsCard** owns the plain thin border and one-cell horizontal inner padding. Titles are left-aligned ACCENT on the border; focused cards add an ACCENT border and `• focused`. Panel content comprises summary/unit, measured chart or table, selected readout and legend. Selection does not alter another panel's size. Only visible panels are drawn.

## TimePlot

The shared `profile_charts` renderer provides count columns, line series, grouped stage bars and signed diverging temperature bars. Time/count data carries bucket boundaries, stable series keys, coverage and selection; duration data retains optional values.

`time_plot` / `coarsen_counts` cover the selected range by summing count buckets and preserving boundaries with common scales. `series_plot` keeps missing duration points as gaps and never averages percentile points. Duration points page explicitly when underlying samples are unavailable. `plot_targets(count, width, selected, series)` returns original bucket index, x offset and cell width; drawing and mouse targets share these values. Rebucket duration data only when authoritative underlying samples/bins are available.

Every plot exposes unit, common zero-based scale where appropriate, range, sparse ticks, legend and selected-point readout. Absolute stacks share a maximum; only explicitly 100% views normalize. Allocate stack cells with largest remainder. `series_style(key)` maps stable identity to color across filters and refreshes. Intel's distinct totals do not sum overlapping tags. p95 is suppressed below the existing sample threshold.

## RankedBars, ComparisonBars and Meter

`rank_bar(value, max, width)`, `stacked_bar(label, label_width, series, bar_cells, total)` and `meter(ratio, width)` share chart rendering. Supply exact values and denominators beside marks; caller-provided common maxima keep panels comparable. Paired stage bars use execution ACCENT and wait WARN, and stack only mutually exclusive values.

```rust,ignore
let mark = charts::meter(Some(accepted as f64 / eligible as f64), 12);
```

**Meter** displays N/A for unknown/disabled denominators. Quota is sends in 60 seconds/effective limit, concurrency active/max and pace requests/minute. Overflow retains its exact numeric value and a warning. Amplification is a multiple (`1.8×`); temperature movement is signed score points.

## DetailTable and Report preset

**DetailTable** renders stable cell-measured columns, aligns numbers, ellipsizes labels and does not wrap dashboard/expanded table rows. Stable-ID row selection survives refresh, filtering and sorting. Full prose and nested authoritative values remain accessible in expanded detail. Do not silently truncate numeric meaning; reduce optional columns or expose complete values through detail.

The **Report** preset occupies approximately 90% of the viewport with independent scrolling and restores the previous Dashboard focus/scroll on Esc. Every chart has a table equivalent, switched with `v`; `s` cycles sort columns. Tables use existing dimension filters, and selected-row detail exposes full prose. Enter on an actionable detail row opens the authoritative owner. Detail lookup is lazy, bounded and cached by filters/revision. Missing historical/provenance facts remain unavailable.

The general `components::detail_table` and `detail_records` helpers remain available for complete labelled-record detail; this prose fallback must not wrap plot/table rows in dashboard panels. Anchored See more appears only when rows are hidden, opens expanded detail and disappears at the bottom.

## ScrollPane

`components::ScrollPane` owns a `usize` offset. `scroll(delta, extent, viewport)` clamps it; `reveal(start, height, viewport)` brings focused content into view; `visible(lines, height)` slices before rendering, avoiding u16 Paragraph offsets. Dashboard, each panel and expanded detail keep independent state.

```rust,ignore
pane.scroll(20, lines.len(), room);
frame.render_widget(Paragraph::new(pane.visible(&lines, room).to_vec()), area);
```

Tab/Shift+Tab uses reading order and reveals the next panel. PgUp/PgDn and wheel scroll focused content then page consistently; keyboard and mouse share geometry. Expanded detail restores its saved dashboard state on Esc.

## ActionBar and Overlay

`components::action_rects(area, labels)` measures labels in terminal cells. Draw and register each returned rectangle. Compact Profile exposes Summary and five app pages through the section control and 0–5 shortcuts. Profile pickers push a layout scope; the top scope owns hit testing and focus.

```rust,ignore
for (target, rect) in targets.iter().zip(action_rects(area, &labels)) {
    registry.register(*target, rect);
}
```

## Measured text

`components::measured_lines(lines, width)` freezes cell and grapheme aware wrapped lines before scrolling. Labels and editor sizing use the same terminal cell measurement. Bracketed paste stays with the current editor.

```rust,ignore
let rows = components::measured_lines(vec![Line::raw(text)], width);
```

## Acceptance

Use [tui-verification.md](tui-verification.md) and `scripts/tui_review.py`. Name the component/preset, assert shared geometry/state behavior and inspect actual fixture PNGs. Capture starts pending; record reviewed images and action/metric results separately.

Check Profile at 160×50, 120×40, 100×32, 80×24, 60×18 and below minimum. Cover all 20 IDs, zero, empty, missing history, loading, stale, error and small samples. Verify simultaneous panels, keyboard/mouse parity, full-record scrolling, state restoration, Unicode/no wrapping, units/scales/denominators and global shortcuts. Render actual terminal fixtures with `scripts/render_tui_cells.py`; compare Summary/Recon composition with approved references when available.

## MeasuredEditor and TranscriptBlock

`components::editor_height(text, width, minimum, maximum)` sizes a wrapped composer. Unicode cell measurement also determines clipping and the displayed cursor position. Existing editor state owns bracketed paste.

```rust,ignore
let height = components::editor_height(&app.composer, width, 1, 8);
```

`TranscriptBlock<T>` stores a content revision, viewport width and frozen rendered content. The Recon frame cache reuses completed Markdown blocks while revision and width remain equal; investigation events arrive through application refresh, so rendering never reads storage.

```rust,ignore
let frozen = TranscriptBlock { revision, width, content: rendered_lines };
```

## Application presets

Home shrinks its logo and uses two launcher columns on short screens. Recon uses a transcript and 30% context column at 110 cells; compact screens expose Transcript/Investigation pages. Intel uses three columns at 132 cells, two at 100–131, and Extracted/Reader/Context pages on smaller or short screens. Its article reader is borderless. Atlas exposes compact Origins/Insights/Headlines pages. Brain supports Split/Graph/Related/Summary expansion. Tools and Models expose list/editor pages; the model role list registers its rows. System exposes Host/Paths pages on short screens.
