# TUI design contract

[Code route](index.md) · [Wiki](../../index.md)

Read this contract and the relevant [component catalog](tui-components.md) entries before changing `crates/argos-osint-bin/src/tui/`. Keep the terminal UI simple: no new chrome, motion, or density modes. React Native presentation follows the [Argos look and feel](react-ui.md#argos-look--feel-reference), where component design may vary inside the theme. That latitude does not transfer here. Profile's current presentation, 20-view inventory and acceptance matrix are specified in [profile-analytics-dashboard.md](profile-analytics-dashboard.md). That document supersedes the older 35-widget and focused-only accordion presentation; telemetry/history and [metric semantics](profile-dashboard-and-search.md#metric-dictionary) remain authoritative.

Name the selected components and layout preset before editing. Extend shared components where necessary, preserve theme tokens and the 20 primary IDs, and use the same rectangles for drawing, focus and pointer targets. Child prompts carry components/preset, exact owned files, read-only references and viewport/interaction/data checks.

## Profile edit map

| Change | Entry points |
| --- | --- |
| Primary registry, Summary, merged details and panel state | `profile.rs`: `WIDGETS`, `WIDGET_COUNT`, `Section`, `draw_overview`, `draw_section_body`, `draw_widget_panel`, `widget_lines` |
| Scales, stacks, axes, legends and units | `profile_charts.rs`: `stacked_bar`, `rank_bar`, `trend_columns`, `meter`, `percent` |
| Pure shared geometry and reusable panels | `profile_layout.rs::LayoutResult`, `profile_components.rs`; module registration in `mod.rs` |
| Focus, scroll, owner activation and mouse | `ui.rs`: Profile branch, `focus_order`, `hit_test`, `system_areas`; `app.rs`: Profile dispatch/tests |
| Snapshot facts and corrected units | Core `profile_stats.rs`: `ProfileSnapshot`, `StatFilters`, `AmplificationBucket::with_ratio`; `provider_metrics.rs::CapacityRow` |
| Theme and configuration regressions | `theme.rs`, `profile_config.rs` |
| User/agent guidance | Dashboard doc, usage/index, AGENTS.md and existing OpenCode skill/agent prompts |

Paths in the table are relative to `crates/argos-osint-bin/src/tui/` except explicit core references. Core snapshot/store APIs and `docs/wiki/code/profile-dashboard-and-search.md` define facts; the presentation does not invent unavailable history or scheduler policy.

## Profile components and presets

Use **AnalyticsCard, TimePlot, DetailTable, Meter and ScrollPane**. `profile_components` is the Profile facade and `components` retains general cell-measured/editor/transcript helpers. The **Dashboard** preset renders simultaneous panels with independent state; the **Report** preset expands detail to approximately 90% of the viewport and restores prior dashboard focus/scroll on Esc.

`profile_layout::LayoutResult` owns the rectangles consumed by drawing, hit testing and focus order. At body width ≥120 use two columns with one-cell gutter; below that use a single column in reading order. KPIs are four across ≥120, two across at 80–119 and compact two-column values below 80. At 160×50, Summary has paired 18-row panels, one blank row, then paired 18-row panels; Recon has paired 11-row panels, one blank row, paired 11-row panels, one blank row and a 13-row full-width unresolved table. See the dashboard doc for fixed chrome and the complete page matrix.

Charts retain an 11-row minimum and tables 9 rows; scroll instead of squeezing. Preserve navigation and display a size notice below 60×18. Focus never collapses other panels. Render visible panels only, ellipsize labels by terminal cells, and never wrap chart/table rows. Full prose remains in detail.

## Theme, data and interaction

Separate complete list records with one blank terminal row by default. Home app
items, Ctrl+K command items, Atlas cycle/article lists and Intel category bulletin
headlines use an explicit zero-gap density. Preserve section headings and section
separation; every other consumer retains spaced records. Keep wrapped
and expanded content together; gaps never count as records or activate adjacent
items. Shared list geometry governs capacity, reveal and pointer mapping, without
trailing overflow. Small panes retain gaps and scroll; charts and horizontal
navigation retain their existing geometry. The Focus Brief reader shows the
stored article description. Retrieval of the full body continues for extraction
and stays off that pane. Messages and orphan legacy running rows cannot
establish retrieval activity. Read failures retain cached content; Reload
invalidates the old presentation.

`theme.rs` remains authoritative: preserve all RGB/semantic tokens, the stable `SERIES` mapping and map colors/heat ramp. Use thin terminal borders, one-cell horizontal padding, ACCENT titles, normal-size bold KPI values, ACCENT navigation fill/dark text and ACCENT focus border with `• focused`. Selected table rows use SELECT/TEXT.

All renderers consume immutable snapshots. Storage, network, model calls and detail lookup belong to background delivery, never draw. Preserve stale data/state during loading; one bounded refresh at a time reuses the existing cadence/connection. Discard obsolete filter generations. Detail reads are lazy, bounded and cached by filters/revision.

Metric contracts distinguish attempts from operations, remote sends from cache hits and verified zero from failure. Show units, eligible denominator/N, coverage and cancellation handling; Live ignores period. Count buckets rebucket by summation; durations/percentiles require underlying samples or bins. Missing durations are gaps; p95 below the existing N threshold is unavailable. Keep shared zero-based scales, largest-remainder stacks, semantic series identities, signed temperature points, amplification multiples, exact overflow and unknown/disabled quotas as N/A.

Tab/Shift+Tab follow top-left→bottom-right and reveal focus; arrows select rows/buckets; Enter expands and detail-row Enter opens owners. PgUp/PgDn/wheel scroll content then page consistently; `v` switches detail chart/table; Esc restores previous focus/scroll. Keep `f` filters, `r` refresh, `?` help, Configs and global shortcuts reachable with keyboard/mouse parity. Picker text and editor keys retain their input owners.

## Venture return and Jobs filters

The no-argument CLI owns a persistent Venture session. Closing its TUI or stopping its owned stack restores the terminal/services and returns to Venture; direct `stack up` commands keep their one-shot behavior. Esc in Venture returns one question and preserves valid values, with incompatible later branch answers invalidated. At the root choice Esc exits. See [Venture navigation](../../diagrams/venture-navigation.html). Each selected session runs in a child process; process exit releases its repository and maintenance workers before the parent menu reopens.

Jobs filter changes are cohort transitions: clear old rows/detail/selection/scroll and pending owner navigation, then show loading until a snapshot for the new key arrives. Reject old keys; keep data during same-filter background refresh. Status/app/search and loaded state participate in action geometry invalidation. Keyboard and mouse filters use the same transition; zero matching rows are a successful empty result, not a loading placeholder.

## Other application presets

The shared data/render separation, geometry, scrolling and measured input apply throughout Argos. Keep orchestration and navigation; a daemon/IPC rewrite is outside UI work.

| Screen | Preset / retained behavior | TUI entry points |
| --- | --- | --- |
| Home | Measured launcher; shrink logo by height; nine apps and composer reachable | `ui.rs::home_layout_metrics`, `draw_home` |
| Service launcher | Centered ARGOS VENTURE composition; core wordmark strokes and shared theme; numbered choice cards, editable ports/URL, settings review; final terminal display; 40×14 minimum | `welcome/terminal.rs::Question`, `wordmark.rs` |
| Recon | Transcript + 30% context at ≥110 columns; compact context page; measured composer | `ui.rs::draw_recon_chat`, `draw_recon_context`, `composer_height`; `recon_parts.rs` |
| Intel | Three columns ≥132, two at 100–131, tabbed below 100 or short height; borderless center stack with bordered Full Article followed by Related; brief Jobs in context | `ui.rs::intel_briefing_areas`, `draw_intel_briefing`, `draw_clipped_md_pane` |
| Atlas | Dashboard/table presets; compact Origins/Coverage/Headlines pages; preserve map | Atlas layout/draw functions; `atlas_table.rs`, `map.rs` |
| Brain | Claims/Actor-name browse projection; Enter on a memory opens the topology mapper at one hop (no detail view); topology Graph first: canvas left 65% / inspector right 35% at ≥132×32, 70%/30% at 100–131×≥32, Graph/Details/Controls pages below; minimum 60×18 | `topology.rs`, `topology_layout.rs`, `ui.rs::draw_brain` |
| Tools / Models | List + editor; compact pages; registered role list | `ui.rs::draw_osint`, `draw_providers`; `model_roles.rs` |
| Jobs / Logs / System | Scrollable tables/details and reachable Host/Paths/Configs actions | `jobs.rs`, `logs.rs`, `profile.rs::draw_system_tab`, `profile_config.rs` |

Retain session state and app identities (`Osint/Providers/System`). Completed transcript Markdown uses revision/width caches rather than database reads during drawing. Overlays own the top input scope; bracketed paste stays with the current editor.

## Agent workflow and acceptance

Follow [tui-verification.md](tui-verification.md) and `argos-tui-verify` for fresh capture and recorded image review. [Session evidence](../../archive/tui-verification-session-2026-10-10.md) demonstrates the procedure. Generated PNGs do not certify actions or metrics.

AGENTS.md and the supported existing OpenCode mechanisms (`argos-plan`, `argos-implement`, `ecc-edit`, `ecc-planner`, `ecc-reviewer`, `ecc-verify`) link these contracts. They preserve disjoint file ownership and concise child prompts. No new OpenCode config key, tool permission or model default is needed.

Verify all 20 primary views, merged details, filters and System/Configs. Render terminal fixtures with `scripts/render_tui_cells.py` at 160×50, 120×40, 100×32, 80×24, 60×18 and too-small, empty, stale and loading states. Check geometry, no wrapping, Unicode, shared scales, denominators, missing/low-N samples, stable colors, owner actions, focus/scroll restoration and mouse parity. Compare actual Summary/Recon snapshots against approved references when available; concept images are not QA.

Run repository fmt → clippy → test with ARGOS_EMBED unset. Agent builds use the isolated wrapper and embedded defaults per [contributor workflow](contributing.md); refresh affected wiki routes/tags/diagrams after code changes. Report actual results and unverified live behavior; acceptance instructions are not evidence that checks have run.


## Shared navigation and portable configuration

TabBar uses three-row bordered buttons when space permits and one-row `[Label]` buttons in compact layouts. ACCENT/bold denotes activation; SELECT denotes keyboard focus independently. Enter/Space activates and Left/Right moves within a focused row. Nested active choices contain `●`; parent pages retain child state. Tab/Shift+Tab follows visual order, with popup scopes trapping focus. Editors retain typing and paste ownership.

Profile has Overview / System / Configs. Overview retains Summary / Intel / Recon / Atlas / Models / Tools and all 20 metric IDs. System and Configs suspend analytics reads and publication. Configs preserves drafts during page navigation and discards credential buffers on explicit Esc. Export defaults to `~/argos-config.json`; a second Export confirms overwriting the same destination. Import uses a multiline editor, Verify, a redacted summary and Save and apply. Verification performs no writes; edits invalidate the exact verified revision. Commit uses the existing transactional snapshot and reloads settings for subsequent work.

Report datasets are bounded DetailTable sections with sticky headers, virtual capacity of 10 logical records on tall screens and 3 on short screens. Spaced data extent is `2*N-1`, plus title/header/footer/gutter, clipped to the viewport. Each section keeps its own row/offset; Tab reveals subsequent sections, wheel/PgUp/PgDn scroll the table then its card at a boundary. Full record prose remains reachable through Enter. Report selection, sort, chart/table mode and dashboard restoration remain separate state.

Intel terminal Bulletin and Focus Brief are text-only. They do not fetch/decode article images, probe terminal graphics capabilities or reserve image columns. Article image URLs remain metadata for the React Native client. Visit article site opens the article HTTP(S) URL. Tools categories toggle independently of search; Documentation and Test response have independent scrolling, with Catalog / Documentation / Test compact pages. Ctrl+K groups Recently used (8 stable IDs), Current app shortcuts and Universal; headings are not selectable.

The brief focus reader uses the full inner width for prose and preserves its source action and scroll behavior.

Recon uses a SURFACE user band with ACCENT edge, quiet Markdown synthesis and expandable evidence activity. Failed activities keep a short reason visible. Transcript offsets are usize, Unicode padding uses terminal cells, completed Markdown keeps revision/width caches, and cached block/local-row anchors preserve history during updates. Jump to latest resumes follow; previous/next-turn commands select user-query boundaries.

Acceptance and reproducible capture: [tui-verification.md](tui-verification.md), tests `dump_overhaul_screens` and `analytics_viewports_and_data_states`. Scope capture to affected apps/pages and states using the verification guide; shared layout changes cover affected consumers at all six viewport sizes including 40×20. Full shell/theme/navigation changes use both suites. Capture into a fresh plan directory and inspect the selected actual PNGs. Live graphics/browser checks are recorded separately from TestBackend evidence.

Intel Focus Brief keeps Inferences as the final middle-column section, after Related. Its measured records, keyboard/page/wheel navigation, mouse targets and topology return use the center stack and Reader compact page; confidence, Actors and Claims stay in Extracted. React Native uses the same final Reader placement after its source link.
