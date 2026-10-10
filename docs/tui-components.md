# TUI components

The [design contract](tui-design-spec.md) preserves the palette in `theme.rs` and all 35 Profile widget IDs. Components render immutable data; storage reads belong to application refresh delivery.

## AnalyticsCard and grid preset

`profile_layout::AnalyticsLayout::new(Rect)` allocates fixed tabs, controls, app selector, content and status. `card(index, offset)` returns only whole visible cards. The grid uses two columns at 104 content cells, four cells between columns, one row between cards, and equal heights clamped to 16–24. Short viewports show a selectable report list.

```rust,ignore
let layout = AnalyticsLayout::new(body);
if let Some(rect) = layout.card(index, offset) {
    registry.register(Target::ProfileCard(index), rect);
    // Render the AnalyticsCard in this same rect.
}
```

`components::analytics_card(frame, area, title, focused)` draws the border and returns its padded content rectangle. Profile's `draw_widget_panel` owns the AnalyticsCard composition: title, period/unit, headline, exact selected bucket, TimePlot and legend. Card borders use the focused accent and one cell of inner padding.

## TimePlot

`profile_charts::TimeBucket` contains `start`, `end`, stable `(series_key, count)` pairs and coverage availability. `time_plot(buckets, width, height, selected)` returns measured plot lines. `coarsen_counts` sums counts and preserves group boundaries; missing history remains unavailable. `SeriesPoint` and `series_plot(points, width, height, selected)` page original duration and ratio observations around selection; missing values remain gaps. They never average percentiles.

```rust,ignore
let plot = charts::time_plot(&buckets, width, height, selected);
```

`series_style(key)` assigns stable categorical colours and semantic outcome colours. Intel plots distinct totals; overlapping tags stay in detail records.

## RankedBars, ComparisonBars and Meter

The existing `rank_bar(value, max, width)`, `stacked_bar(label, label_width, series, bar_cells, total)` and `meter(ratio, width)` APIs share the chart renderer. Callers supply exact values and denominators beside the marks. Pair comparison bars by explicit series identity; stack mutually exclusive values only.

```rust,ignore
let mark = charts::meter(Some(accepted as f64 / eligible as f64), 12);
```

## DetailTable and report preset

`components::detail_table(value, columns, width)` renders complete cell widths with numeric alignment when all columns fit, otherwise uses labelled records. `detail_records(value, width)` retains nested fields and renders null as N/A. Neither truncates numeric values. `ReportLayout::new(inner, selected_height)` places detail beside the plot at 120 useful cells, otherwise below it; short layouts retain authoritative details first.

```rust,ignore
let lines = components::detail_table(&data, widget.detail_columns, width);
```

## ScrollPane

`ScrollPane` owns a `usize` offset. `scroll(delta, extent, viewport)` clamps it; `reveal(start, height, viewport)` brings a whole card into view; `visible(lines, height)` slices before rendering, avoiding u16 Paragraph offsets. Grid and report keep independent scroll state.

```rust,ignore
pane.scroll(20, lines.len(), room);
frame.render_widget(Paragraph::new(pane.visible(&lines, room).to_vec()), area);
```

## ActionBar and Overlay

`components::action_rects(area, labels)` measures labels in terminal cells. Draw and register each returned rectangle. Compact Profile uses a current-app control with the six app choices accessible through its action and 0–5 shortcuts. Profile pickers push a layout scope; the top scope owns hit testing and focus.

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

Check Profile at 160×50, 120×40, the 104/103 content boundary, 100×36, 80×24, 40×20 and 100×24. Cover zero, empty, missing history, loading, stale, error and small samples. Verify keyboard and mouse reachability, full-record scrolling and global shortcuts.

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
