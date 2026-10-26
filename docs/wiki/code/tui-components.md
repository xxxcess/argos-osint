# TUI components

[Code route](index.md) · [Wiki](../../index.md)

The [design contract](tui-design-spec.md) preserves the palette in `theme.rs`. Profile follows the [20-view dashboard contract](profile-analytics-dashboard.md) and retains metric semantics/history in [profile-dashboard-and-search.md](profile-dashboard-and-search.md). Components render immutable data; storage reads belong to background refresh/detail delivery.

## Service launcher

`welcome/terminal.rs` presents **ARGOS VENTURE** using the core wordmark glyph style and shared `wordmark::paint_logo` stroke palette. It reuses `theme::panel`, semantic text/focus styles and Ratatui cell measurements. The measured composition, question card, choices, editable value, settings summary and controls are centered. Small terminals use a compact text title. Each question owns one choice list or text editor and a scrollable settings summary. The same measured choice rows drive mouse activation and rendering; click commits on mouse-up within one cell of press. Keyboard selection, validation, Unicode cursor/editing, paste, cancellation, resize and below-minimum behavior are covered by focused launcher tests. It opens no datastore; the resulting selection dispatches the existing stack commands after restoring the terminal.

Server traffic uses existing Jobs service rows and Jobs → View logs navigation. Running API/Web service rows sort ahead of ordinary work within the selected filters, keeping them visible on crowded bounded pages; other service rows retain their normal ordering. Work-count headers exclude services. Renderers consume the normal `JobRow` / `EventRow` snapshots; API and frontend producers attach redacted events through a bounded queue rather than writing terminal escape sequences into the app. The repository limits the ID page before loading correlated event/task/attempt details.

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

Logical records have one blank terminal row between complete items by default, with no
trailing gap. `components::LIST_ITEM_GAP_ROWS` and `ListGeometry` define content
ranges, extent, capacity, title-first reveal and row-to-item mapping. Wrapped
titles, supporting metadata and expanded details stay contiguous. Spacer rows
use ordinary pane backgrounds and have no item target; boxed tables retain
vertical strokes. Counts and selection indices remain logical. Single-line
capacity is `(available_rows+1)/2`; two-line Brain capacity is
`(available_rows+1)/3`. Home, roles and fallbacks keep independent offsets.
Markdown display spacing leaves stored text, ordinary paragraphs and code intact.

Explicit `ListGeometry::{with_gap,fixed_with_gap,capacity_with_gap}` consumers
use zero gap only for Home app rows, command palette items, Atlas cycle/article
lists and Intel category headlines. Their draw/capacity/reveal/hit geometry shares
the same density. Section headings remain nonselectable and retain their section
separation. Other lists, including Brain and topology relationships, keep the
default gap. Dense extent sums record heights; a variable gap adds `gap*(N-1)`.

Full article presentation derives from the selected article/current job snapshot.
Nonempty committed Body Markdown supersedes legacy flags/messages and later stage
activity. Usable complete/partial/uncertain bodies retain quality warnings.
Only confirmed running Body work without usable content animates; queued, retry,
terminal and read-error states expose status and Reload. Snapshot acceptance
assigns body/stages/outputs together and clears obsolete retrieval tracking.
Read failures preserve cached content. Navigation rejects mismatched owners;
Reload invalidates previous-generation content. Drawing performs no reads.

Intel Focus Brief keeps its center stack borderless and uses the shared clipped border, padding, independent scroll limits and See more geometry for the **summary** card, which shows the stored description rather than the retrieved article body. Its six-row virtual minimum reserves two borders, a readable content row and the three-row See more action without overlap. Confidence keeps its ANSI Shadow badge with three fields: Source, Information, Brief rating. Actors from immutable metadata appear above Claims and remain visible without claims and during refresh. Claim and Inference records use measured whole-record targets to open their exact topology, with the same order for drawing, focus and pointer actions. The context column contains Tags, Country and brief Jobs. Historical article reports remain stored; their execution and Full Assessment Recon controls are removed. Jobs action geometry includes global **Clear completed**, **Clear stopped** and **Clear failed**, plus selected **Stop job** for supported running/queued/retry work, with matching keyboard and pointer routes. The Jobs ActionBar uses one-row controls on short bodies to retain table/detail space; wider/taller layouts retain bordered buttons. Service detail explains long-lived availability and distinguishes service age from task work.

The brief **Jobs** pane shows the dependency chain summary followed by one spaced record per stage (Full article, Claims, Inferences, Links). Active stages spin; Full Article and Claims may run together, then Inferences joins both and Links follows. Running rows name their activity: retrieving / cleaning, waiting for model, validating, or checking fidelity. Queued rows name their blocker; failed rows show a short reason. A **Publication / Index** footer follows: Pending, Published · Index pending / Indexed / Not indexed · reason / Index failed · reason, with historical reports excluded from precedence. Global Jobs shows the same `Stage · row` labels in stage order, and each committed stage logs `intel_brief.stage`. Content panes show completed checkpoints as soon as they commit, and they never inherit a parent failure: validated artifacts read "Validated · not published". Links render as **Potential corroboration**. Captures: `intel_sequential_stage_screens` at 160×50, 120×40, 100×32, 80×24, 60×18 and 40×12; images are in `docs/screens/intel-sequential/`. The [claim context verification](../../screenshots/intel-claim-context-2026-10-16/README.md) records fresh Actors-first, Inference-entry and staged-publication review.

## Brain Memory Topology and GraphCanvas

`topology_layout::TopologyLayoutResult` computes pure geometry for the **Graph first** preset (dimensions refer to its available body):
- **Desktop (≥132×32)**: GraphCanvas left 65%, TopologyInspector right 35%.
- **Compact (100–131 columns, ≥32 rows)**: GraphCanvas 70%, TopologyInspector 30%.
- **Stacked (<100 columns or <32 rows)**: Full-width `[Graph] [Details] [Controls]` TabBar pages.
- **Tiny (<60×18 cells)**: Explicit minimum size guidance panel.

The inspector body has **Summary** / **Record** `TabBar` content tabs (Summary default; `ContentTab` state, restored by history). Summary draws styled lines built from the typed brief (cached per basis/generation/outcome/width) in a `ScrollPane` with measured geometry, an overflow indicator, a wrapped state label and failure actions (Retry summary, View logs, Models). Typed basis rows are `Target::TopologyBasis` and open Record on activation. `compute_with(area, summary_active)` gives the Summary details panel 75% of inspector height (min 8, Relationships keeps ≥5).

`topology.rs` renders only immutable scoped snapshot nodes, edges and receipts;
no sample graph or evidence substitutes for loading, empty or failed data.
GraphCanvas uses deterministic hop columns and cell-measured clipped targets
shared by rendering, focus and pointer registration. Actor groups preserve
member identity; Inference is visible; Topic is a recorded-attribute projection,
not a new canonical memory kind. Glyphs/colors use existing theme tokens.

Selection changes the inspector without changing the seed. Recenter (or `c`)
uses the selected node's exact typed seed and pushes history. Relationships show
true direction, status, rationale and receipts; Related remains tentative possible
context. Inspector relationships retain one blank row and independent scrolling.
Open record shows complete snapshot detail. Back restores history, then origin.
Depth/scope/counts reflect state; Graph/Details/Controls separate focus from
activation. Zoom/pan/center affect positions. Reset view clears the transform;
crowded virtual hop columns support wheel/Page scrolling and Shift+Arrow pan. Legend and visible/omitted coverage stay
below the canvas. Decorative minimaps and inert navigation drawers are omitted.

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

For shared Profile geometry/registry changes, check Profile at 160×50, 120×40, 100×32, 80×24, 60×18 and below minimum, covering all 20 IDs. For local changes, select affected views, thresholds and zero/empty/missing/loading/stale/error/small-sample states per the verification guide. Verify simultaneous panels, keyboard/mouse parity, full-record scrolling, state restoration, Unicode/no wrapping, units/scales/denominators and global shortcuts. Render actual terminal fixtures with `scripts/render_tui_cells.py`; compare Summary/Recon composition with approved references when available.

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

Intel Bulletin previews and Focus Brief prose use the full text width. Short previews remain vertically centered and overflowing previews retain normal scrolling. Article media is displayed by the React Native Intel feed, not the terminal.

Home shrinks its logo and uses two launcher columns on short screens. Recon uses a transcript and 30% context column at 110 cells; compact screens expose Transcript/Investigation pages. Intel uses three columns at 132 cells, two at 100–131, and Extracted/Reader/Context pages on smaller or short screens. Its center stack is borderless, with bordered summary and Related cards in every state. The summary card shows the stored description. Atlas exposes compact Origins/Coverage/Headlines pages. Brain Claims support Split/Graph/Related/Summary expansion; Actor groups use Graph/Related, 65%/35% at ≥100×28 and compact pages otherwise. Tools and Models expose list/editor pages; the model role list registers its rows. System exposes Host/Paths pages on short screens.


## Shared navigation and portable configuration

TabBar uses three-row bordered buttons when space permits and one-row `[Label]` buttons in compact layouts. ACCENT/bold denotes activation; SELECT denotes keyboard focus independently. Enter/Space activates and Left/Right moves within a focused row. Nested active choices contain `●`; parent pages retain child state. Tab/Shift+Tab follows visual order, with popup scopes trapping focus. Editors retain typing and paste ownership.

Profile has Overview / System / Configs. Overview retains Summary / Intel / Recon / Atlas / Models / Tools and all 20 metric IDs. System and Configs suspend analytics reads and publication. Configs preserves drafts during page navigation and discards credential buffers on explicit Esc. Export defaults to `~/argos-config.json`; a second Export confirms overwriting the same destination. Import uses a multiline editor, Verify, a redacted summary and Save and apply. Verification performs no writes; edits invalidate the exact verified revision. Commit uses the existing transactional snapshot and reloads settings for subsequent work.

Report datasets are bounded DetailTable sections with sticky headers, virtual capacity of 10 logical records on tall screens and 3 on short screens. Spaced data extent is `2*N-1`, plus title/header/footer/gutter, clipped to the viewport. Each section keeps its own row/offset; Tab reveals subsequent sections, wheel/PgUp/PgDn scroll the table then its card at a boundary. Full record prose remains reachable through Enter. Report selection, sort, chart/table mode and dashboard restoration remain separate state.

Intel terminal Bulletin and Focus Brief are text-only. They do not fetch/decode article images, probe terminal graphics capabilities or reserve image columns. Article image URLs remain metadata for the React Native client. Visit article site opens the article HTTP(S) URL. Tools categories toggle independently of search; Documentation and Test response have independent scrolling, with Catalog / Documentation / Test compact pages. Ctrl+K groups Recently used (8 stable IDs), Current app shortcuts and Universal; headings are not selectable.

Recon uses a SURFACE user band with ACCENT edge, quiet Markdown synthesis and expandable evidence activity. Failed activities keep a short reason visible. Transcript offsets are usize, Unicode padding uses terminal cells, completed Markdown keeps revision/width caches, and cached block/local-row anchors preserve history during updates. Jump to latest resumes follow; previous/next-turn commands select user-query boundaries.

Acceptance and reproducible capture: [tui-verification.md](tui-verification.md), tests `dump_overhaul_screens` and `analytics_viewports_and_data_states`. Scope capture to affected apps/pages and states using the verification guide; shared layout changes cover affected consumers at all six viewport sizes including 40×20. Full shell/theme/navigation changes use both suites. Capture into a fresh plan directory and inspect the selected actual PNGs. Live graphics/browser checks are recorded separately from TestBackend evidence.

## Related article rows

`related_articles::ArticleRelatedView` supplies immutable bounded Related pages for Intel Focus Brief. It sits beneath the summary card in the middle column, with independent cursor/scroll, a focused border, wrapped titles, clipped geometry and anchored See more. Scope remains All retained cycles / This cycle. Drawing, focus, reveal and pointer targets share the center-stack layout. Stale or missing authoritative article revisions cannot open another row.

Brain has no detail view: Enter on a memory (or actor group) opens its Memory Topology at one hop, and Back returns to the list with selection and scroll intact. Intel extracted claims and actors retain measured per-record targets and source identity. Actor totals remain local; a single-member AI summary is never shown for a group.

Related article eligibility requires **at least two distinct overlapping entity mentions** in the exact source/candidate headline or description. Repetitions and aliases of one normalized name count once; country/category fields do not count as mentions. Atlas ingest persists Related associations; Intel Related reflects those retained pairs and rechecks current revisions. Brain Actor Articles counts direct retained mentions, observed links and claim origins without requiring a two-entity pair. A browse visit adds no name-only or canonical-actor suggestions. Original claim Sources preserve provenance independently. Investigation entity associations require Recall on; later disabling Recall preserves earlier receipts. The local match receipt is `shared-headline-entities-v4-possessive`; changing this policy invalidates only Related matching and the dependent Links shortlist, preserving compatible Body/Claims/Inferences and fidelity checkpoints.

Production Brain list/detail and Intel Related use background snapshots. Cached list filtering and graph/tab rendering perform no database work; stale request results are discarded. A refreshing list retains its previous rows, selection, Find and scroll. Background snapshot ownership and the database/index audit are described in [tui-database-responsiveness.md](tui-database-responsiveness.md).
