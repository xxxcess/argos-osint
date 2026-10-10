# TUI design contract

Read this contract and the relevant [component catalog](tui-components.md) entries before changing `crates/argos-osint-bin/src/tui/`. Profile's current presentation, 20-view inventory and acceptance matrix are specified in [profile-analytics-dashboard.md](profile-analytics-dashboard.md). That document supersedes the older 35-widget and focused-only accordion presentation; telemetry/history and [metric semantics](profile-dashboard-and-search.md#metric-dictionary) remain authoritative.

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

Paths in the table are relative to `crates/argos-osint-bin/src/tui/` except explicit core references. Core snapshot/store APIs and `docs/profile-dashboard-and-search.md` define facts; the presentation does not invent unavailable history or scheduler policy.

## Profile components and presets

Use **AnalyticsCard, TimePlot, DetailTable, Meter and ScrollPane**. `profile_components` is the Profile facade and `components` retains general cell-measured/editor/transcript helpers. The **Dashboard** preset renders simultaneous panels with independent state; the **Report** preset expands detail to approximately 90% of the viewport and restores prior dashboard focus/scroll on Esc.

`profile_layout::LayoutResult` owns the rectangles consumed by drawing, hit testing and focus order. At body width ≥120 use two columns with one-cell gutter; below that use a single column in reading order. KPIs are four across ≥120, two across at 80–119 and compact two-column values below 80. At 160×50, Summary has paired 18-row panels, one blank row, then paired 18-row panels; Recon has paired 11-row panels, one blank row, paired 11-row panels, one blank row and a 13-row full-width unresolved table. See the dashboard doc for fixed chrome and the complete page matrix.

Charts retain an 11-row minimum and tables 9 rows; scroll instead of squeezing. Preserve navigation and display a size notice below 60×18. Focus never collapses other panels. Render visible panels only, ellipsize labels by terminal cells, and never wrap chart/table rows. Full prose remains in detail.

## Theme, data and interaction

`theme.rs` remains authoritative: preserve all RGB/semantic tokens, the stable `SERIES` mapping and map colors/heat ramp. Use thin terminal borders, one-cell horizontal padding, ACCENT titles, normal-size bold KPI values, ACCENT navigation fill/dark text and ACCENT focus border with `• focused`. Selected table rows use SELECT/TEXT.

All renderers consume immutable snapshots. Storage, network, model calls and detail lookup belong to background delivery, never draw. Preserve stale data/state during loading; one bounded refresh at a time reuses the existing cadence/connection. Discard obsolete filter generations. Detail reads are lazy, bounded and cached by filters/revision.

Metric contracts distinguish attempts from operations, remote sends from cache hits and verified zero from failure. Show units, eligible denominator/N, coverage and cancellation handling; Live ignores period. Count buckets rebucket by summation; durations/percentiles require underlying samples or bins. Missing durations are gaps; p95 below the existing N threshold is unavailable. Keep shared zero-based scales, largest-remainder stacks, semantic series identities, signed temperature points, amplification multiples, exact overflow and unknown/disabled quotas as N/A.

Tab/Shift+Tab follow top-left→bottom-right and reveal focus; arrows select rows/buckets; Enter expands and detail-row Enter opens owners. PgUp/PgDn/wheel scroll content then page consistently; `v` switches detail chart/table; Esc restores previous focus/scroll. Keep `f` filters, `r` refresh, `?` help, Configs and global shortcuts reachable with keyboard/mouse parity. Picker text and editor keys retain their input owners.

## Other application presets

The shared data/render separation, geometry, scrolling and measured input apply throughout Argos. Keep orchestration and navigation; a daemon/IPC rewrite is outside UI work.

| Screen | Preset / retained behavior | TUI entry points |
| --- | --- | --- |
| Home | Measured launcher; shrink logo by height; nine apps and composer reachable | `ui.rs::home_layout_metrics`, `draw_home` |
| Recon | Transcript + 30% context at ≥110 columns; compact context page; measured composer | `ui.rs::draw_recon_chat`, `draw_recon_context`, `composer_height`; `recon_parts.rs` |
| Intel | Three columns ≥132, two at 100–131, tabbed below 100 or short height; borderless reader and selected BLUF | `ui.rs::intel_briefing_areas`, `draw_intel_briefing`, `draw_clipped_md_pane`; `summary_card.rs` |
| Atlas | Dashboard/table presets; compact Origins/Insights/Headlines pages; preserve map | Atlas layout/draw functions; `atlas_table.rs`, `map.rs` |
| Brain | Graph above Related/Summary; side-by-side ≥68, otherwise stacked; pane expansion | `brain_detail.rs`, `graph.rs`, `ui.rs::draw_brain` |
| Tools / Models | List + editor; compact pages; registered role list | `ui.rs::draw_osint`, `draw_providers`; `model_roles.rs` |
| Jobs / Logs / System | Scrollable tables/details and reachable Host/Paths/Configs actions | `jobs.rs`, `logs.rs`, `profile.rs::draw_system_tab`, `profile_config.rs` |

Retain session state and app identities (`Osint/Providers/System`). Completed transcript Markdown uses revision/width caches rather than database reads during drawing. Overlays own the top input scope; bracketed paste stays with the current editor.

## Agent workflow and acceptance

Follow [tui-verification.md](tui-verification.md) and `argos-tui-verify` for fresh capture and recorded image review. [Session evidence](tui-verification-session-2026-10-10.md) demonstrates the procedure. Generated PNGs do not certify actions or metrics.

AGENTS.md and the supported existing OpenCode mechanisms (`argos-plan`, `argos-implement`, `ecc-edit`, `ecc-planner`, `ecc-reviewer`, `ecc-verify`) link these contracts. They preserve disjoint file ownership and concise child prompts. No new OpenCode config key, tool permission or model default is needed.

Verify all 20 primary views, merged details, filters and System/Configs. Render terminal fixtures with `scripts/render_tui_cells.py` at 160×50, 120×40, 100×32, 80×24, 60×18 and too-small, empty, stale and loading states. Check geometry, no wrapping, Unicode, shared scales, denominators, missing/low-N samples, stable colors, owner actions, focus/scroll restoration and mouse parity. Compare actual Summary/Recon snapshots against approved references when available; concept images are not QA.

Run repository fmt → clippy → test with ARGOS_EMBED unset. OpenCode build/clippy/test use `--locked --no-default-features` per [AGENTS.md](../AGENTS.md), and graphify is updated after code changes. Report actual results and unverified live behavior; acceptance instructions are not evidence that checks have run.
