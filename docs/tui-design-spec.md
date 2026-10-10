# Argos OSINT TUI: concise implementation handoff

## 1. Goal and scope

Redesign Profile analytics around Codex CLI’s usage dashboard: aligned cards, readable time plots, precise selected-period details, and maximized reports. Preserve Argos’s current colors. Establish shared components and layout presets so subsequent UI changes follow the same rules.

Audited Argos `090dc5bf8b0c952cc58bdec8bb5f8608deb10043`, including the newly pushed Profile implementation. Codex reference: `c3d3b142d10f4316b46e35aad7e5317e7e506cb7`. Recheck the implementation diff if main advances. This is an implementation handoff; product code has not been changed.

Adapt the supplied TUI baseline’s separation of data and rendering, shared geometry, scrolling, typed content, and measured input. Keep Argos’s existing orchestration and navigation model; a daemon/IPC rewrite is outside this UI work.

**Install this file as `docs/tui-design-spec.md`.** Read only this spec, the relevant component catalog entries, and the files for the current phase.

## 2. Edit map: start here

Paths are relative to the repository root. Edit the current phase's files only; functions below are search anchors, avoiding broad repository exploration.

| Change | Files / entry points |
| --- | --- |
| Dashboard, widget registry, report state, controls | `crates/argos-osint-bin/src/tui/profile.rs`: `ProfileView`, `WidgetSpec`, `WIDGETS`, `draw_section_body`, `draw_widget_panel`, `widget_lines`, `handle_key` |
| Chart primitives, scales, labels, numeric formatting | `crates/argos-osint-bin/src/tui/profile_charts.rs`: `trend_columns`, `stacked_bar`, `rank_bar`, `meter` |
| Geometry and shared component APIs | **Create** `crates/argos-osint-bin/src/tui/profile_layout.rs` and `crates/argos-osint-bin/src/tui/components.rs`; wire them in `crates/argos-osint-bin/src/tui/mod.rs` |
| Focus, shortcuts, scrolling, refresh delivery | `crates/argos-osint-bin/src/tui/app.rs`: `LayoutRegistry`, `Target`, `Region`, `handle_key`, `handle_mouse`, `reload_profile` |
| Shell integration, hit-testing, help/hints | `crates/argos-osint-bin/src/tui/ui.rs`: `chrome`, `focus_order`, `hit_test`, `region_at`, `system_hit`, `draw_system`, and Profile hint/help cases |
| Presentation documentation | **Create** `docs/tui-design-spec.md`, `docs/tui-components.md`; update `docs/profile-dashboard-and-search.md`, `docs/usage.md`, `docs/conventions.md`, `docs/README.md` |
| Agent workflow | Exact `AGENTS.md` and `.opencode/` files are listed in §7 |
| Verification | Existing inline test modules in `profile.rs`, `profile_charts.rs`, `app.rs`, `ui.rs`; place geometry tests alongside the new modules |

**Read for data semantics:** `crates/argos-osint-core/src/profile_stats.rs` (`ProfileSnapshot`, `StatFilters`, `Period`, `snapshot`); `crates/argos-osint-core/src/store.rs` (`profile_snapshot`, `profile_filter_options`); `docs/profile-dashboard-and-search.md` (metric dictionary). These are the starting sources, not a request to rewrite telemetry or schema. Keep `profile_config.rs` as the Configs integration reference.

## 3. Current audit → required fixes

TUI paths below are relative to `crates/argos-osint-bin/src/tui/`.

| Evidence | Implementation requirement |
| --- | --- |
| `profile.rs::draw_section_body` expands one widget; other widgets become titles | Replace the accordion with an overview grid and full reports |
| `profile_charts.rs::trend_columns` has no dates, axes, totals, or selection; `volume_lines` caps width at 48 | Use a measured plot with baseline, scale, time labels, cursor, and exact detail |
| `see_more` increases rows up to 64; `draw_widget_panel` still clips them | Reports must scroll through the full dataset |
| `widget_lines` forces a minimum width of 20; numeric rows are wrapped Paragraphs | Fit the actual rectangle; use compact records when columns cannot fit |
| Filter state has no rendered picker; period is not editable; all six dimensions are not reachable | Add registered period/filter controls and a real picker |
| Profile handles bare-letter cases without checking modifiers before global shortcuts | Global shortcuts first; module commands require their intended modifiers |
| Overview controls lack registered targets; `system_hit` uses geometry predating Profile’s inner chrome | Share rectangles for drawing, focus, click, and wheel handling |
| Snapshot refresh is synchronous; a last-good snapshot hides reload errors | Load off the UI path; display stale data and the latest error visibly |
| Profile help/footer still describe hardware only | Update contextual hints, help, and usage docs |

The existing metric layer is valuable: retain `ProfileSnapshot`, the 35 stable widget IDs, aggregation semantics, and configuration import/export behavior.

## 4. Profile target

### Navigation and layout

Keep **Overview / System**. Overview starts at **All apps**, followed by Intel, Recon, Atlas, Models, Tools. App selection changes the displayed reports; it is separate from the telemetry `app` filter.

Fixed chrome: tab row → period/filter row → app selector → scrollable content → one status/hint row. On short screens combine controls into a compact action row with a picker. Keep the selected app and active filters visible.

**All apps:** five summary cards, one per app, using the registered primary widgets below. Each card has title, period/group/unit, headline total, plot, selected-bucket callout, and a short legend. Enter opens its full report. App views expose all existing widgets as cards in registry priority order.

**Grid preset:** two columns when content width ≥104; otherwise one. Use a four-cell horizontal gutter, one blank row between card rows, one-cell inner padding, and Argos’s existing plain borders. Card height is viewport-derived, clamped to 16–24 rows, and remains identical across loading, errors, zero values, and selection. Scroll the grid; moving focus brings the whole card into view. If even one card cannot fit, use a selectable metric list that opens reports.

**Report preset:** full-width plot and detail table; at ≥120 columns place the detail beside the plot only when both retain useful widths. Keep controls fixed while the report scrolls. Esc restores the grid’s focus and scroll position.

### Primary cards and chart mapping

| App | Primary widget / headline | Other registered reports |
| --- | --- | --- |
| Intel | `intel.volume`: sum distinct article bucket totals; single-series time bars | `confidence`: paired comparison; `origins`, `publishers`: ranked bars; `freshness`: histogram; `enrichment`, `reports`: tables |
| Recon | `recon.outcomes`: recorded terminal runs; outcome time stacks | `stages`: execution/wait comparison; `recall`, `diversity`, `directives`: rates with counts; `workload`, `unresolved`: tables |
| Atlas | `atlas.cycles`: terminal cycles; outcome time stacks | `hot_zones`: ranked origins; `temperature`: signed comparisons; `cycle_time`: duration plot; `discovery`: disposition stacks; `backlog`: stage records |
| Models | `models.by_role`: actual wire sends; role time stacks | `capacity`: live meters; `latency`, `queue`: duration plots; `performance`, `fallback`: tables; `amplification`: ratio plot; `failures`: cause time stacks |
| Tools | `tools.outcomes`: recorded invocation outcomes; outcome time stacks | `usage`, `attribution`, `failure_causes`: ranked/stacked bars; `reliability`, `search_health`, `evidence`: tables with rate meters |

Report suffixes use their app prefix. Keep **7 Intel + 7 Recon + 6 Atlas + 8 Models + 7 Tools = 35**. Overview cards reference those IDs; they do not create additional metric definitions.

### Plot and data rules

- Plot bars use available width, spaced columns, fractional block heights, a zero baseline, and a readable 1/2/5 scale. Show totals above bars when they fit, sparse complete time labels, and a selected-bucket marker. Never wrap a chart as prose.
- Keep exact value, full timestamp, unit, series values, and N in the selected detail. When space is insufficient, remove decorative labels or the plot before removing authoritative details.
- Cover the entire selected window. Coarsen **count** buckets by summing when necessary; retain bucket boundaries. Duration/ratio points use paging or correctly recomputed aggregates. Never average percentiles or silently drop old buckets.
- Stack only mutually exclusive categories. Intel multi-tag counts overlap: plot distinct totals and show tags separately. Keep cancelled/unknown categories where the metric contract includes them.
- Preserve the metric dictionary in `docs/profile-dashboard-and-search.md`: attempts ≠ operations; cache hits ≠ remote requests; failed transport ≠ verified zero. Show denominators and small-sample labels; suppress p95 below the existing N threshold.
- Separate measured zero, empty window, unavailable history, loading, stale data, and error. Respect `observed_since` and retention; do not interpolate missing durations or invent earlier activity.
- Capacity, active jobs, and queue counts are labelled **Live**, independent of historical filters. Indicate filters unsupported by a report.
- Keep a last-good snapshot during refresh. One bounded background read at a time, approximately 1 Hz while visible; tag results with filter/range generation and discard obsolete responses. Render from immutable data only.

### Interaction

Tab/Shift+Tab traverse registered controls; activate Overview/System with Enter or a documented `t` shortcut. This deliberately replaces Profile’s Tab-to-switch behavior. Preserve `[`/`]` and local 1–5 for app views; All apps has its own registered control.

In the grid, arrows or j/k select cards; Enter opens a report; `m` remains an alias for expansion. In a report, Left/Right selects buckets; PageUp/PageDown and wheel scroll the focused pane. `p` selects 1h/24h/7d/30d; `f` opens all six dimension filters; `c` clears dimensions while preserving period. Typing inside pickers belongs to the field.

Overview `r` refreshes statistics; System refreshes hardware through its own action. Preserve Configs and Esc navigation. Ctrl+K, Ctrl+Q, and text-editing keys must reach their global/editor owners. Register mouse targets for tabs, selectors, cards, plot buckets, filters, and actions.

## 5. Predefined components and presets

Extend the existing chart module; create `profile_layout.rs` for analytics geometry and a small `components.rs` facade around shared TUI presentation. These are proposed modules, not existing APIs.

| Component | Required contract |
| --- | --- |
| `AnalyticsCard` | Stable height; title, summary, plot, selected detail, legend; focused accent border |
| `TimePlot` | Explicit bucket boundaries, unit, optional values, selection and coverage; count bars or duration/ratio series |
| `RankedBars / ComparisonBars / Meter` | Common label/value columns; explicit denominator; exact values alongside marks |
| `DetailTable` | Stable numeric columns, cell-aware truncation, compact labelled-record fallback |
| `ScrollPane` | Independent per-view offset/extent using `usize`; scroll position/overflow indicator; bounded visible rendering |
| `ActionBar / Overlay` | Measured labels, wrapping/overflow picker; topmost input scope owns focus |
| `MeasuredEditor / TranscriptBlock` | Wrapped input sizing and bracketed paste; typed, revision-cached transcript content |

Extend `WidgetSpec` with renderer kind, unit, stable series keys, applicable filters, and detail columns. Widgets provide data and parameters; shared components own geometry and drawing. Add `docs/tui-components.md` with actual APIs, presets, and one minimal usage example per component.

**Theme:** `theme.rs` is authoritative. Preserve BG `#141414`, ACCENT `#87BFFF`, all other semantic tokens, the newly added eight-color `SERIES`, and existing map colors/heat ramp. Map series by stable identity across filters and refreshes; outcomes use consistent semantic colors. Match legend and plot colors. Retain plain Argos borders, text labels, and focus marks. Codex supplies the layout pattern; Argos supplies the palette.

## 6. Apply the baseline across Argos

Use one geometry result for render, hit-testing, and focus order. Fix overlay scope separation and replace raster-scanned focus discovery with registered targets. Use cell/grapheme-aware text measurement, independent scrolling, and reusable components.

| Screen | Preset / retained behavior | Later edit entry points (TUI directory) |
| --- | --- | --- |
| Home | Measured launcher; shrink logo by height; nine apps and composer reachable | `ui.rs::home_layout_metrics`, `draw_home` |
| Recon | Transcript + 30% context at ≥110 columns; compact context page; measured composer | `ui.rs::draw_recon_chat`, `draw_recon_context`, `composer_height`; `recon_parts.rs` |
| Intel | Three columns ≥132, two at 100–131, tabbed below 100 or short height; borderless reader and selected BLUF | `ui.rs::intel_briefing_areas`, `draw_intel_briefing`, `draw_clipped_md_pane`; `summary_card.rs` |
| Atlas | Dashboard/table presets; compact Origins/Insights/Headlines pages; preserve map behavior | `ui.rs` Atlas layout/draw functions; `atlas_table.rs`, `map.rs` |
| Brain | Graph above Related/Summary; side-by-side ≥68, otherwise stacked; pane expansion | `brain_detail.rs`, `graph.rs`, `ui.rs::draw_brain` |
| Tools / Models | List + editor; compact pages; registered role list | `ui.rs::draw_osint`, `draw_providers`; `model_roles.rs` |
| Jobs / Logs / System | Scrollable tables/details; reachable actions, Host, Paths, Configs | `jobs.rs`, `logs.rs`, `profile.rs::draw_system_tab`, `profile_config.rs` |

Retain session state, existing app identities (`Osint/Providers/System`), and background investigation behavior. Transcript performance work follows Profile: revision-based per-block caches, frozen completed Markdown, and no database reads inside drawing.

## 7. Agent and OpenCode updates

Add this concise rule to **AGENTS.md**:

> For changes under crates/argos-osint-bin/src/tui/, read docs/tui-design-spec.md and the relevant docs/tui-components.md entries. Name the components and layout preset before editing. Extend shared components when required. Preserve theme tokens, widget IDs, and metric semantics. Drawing, focus, and mouse targets must share geometry. Verify relevant viewport snapshots and interaction/data states. Child prompts carry chosen components, preset, owned files, read-only references, and acceptance checks.

| Existing file | Required update |
| --- | --- |
| `.opencode/skills/argos-plan/SKILL.md` | UI phases identify components/preset, affected screens, and visual acceptance criteria |
| `.opencode/skills/argos-implement/SKILL.md` | Load relevant catalog entries; pass concise component contracts to each editor; preserve disjoint file ownership |
| `.opencode/agents/ecc-edit.md` | Read supplied UI references; reuse components; report catalog/API changes |
| `.opencode/agents/ecc-planner.md`, `.opencode/agents/ecc-reviewer.md` | Planner uses presets; reviewer checks geometry, data semantics, reachability, palette, and snapshots |
| `docs/conventions.md`, `docs/README.md`, `docs/opencode-v2.md`, `docs/usage.md` | Link the contract/catalog and replace conflicting focus, Profile, and shortcut descriptions |

Current `.opencode/opencode.json` already allows reading project files. This workflow needs no new skill allowlist, tool permission, or model change. Enforce it through AGENTS.md and the existing skills/prompts; avoid copying the entire spec into every prompt.

## 8. Implementation order and completion checks

1. **Contract:** install spec/catalog and agent references; preserve metric inventory and theme.
2. **Foundation:** `components.rs`, `profile_layout.rs`, `profile_charts.rs`, `mod.rs`, then `app.rs::LayoutRegistry` / `ui.rs::focus_order`; shared geometry, scrolling, measured labels, chart API and WidgetSpec.
3. **Profile:** `profile.rs`, `app.rs::reload_profile` / input handlers, and `ui.rs` integration; All apps grid, app views, full reports, controls, bucket details, async refresh/stale states.
4. **Verify Profile:** deterministic TestBackend fixtures at 160×50, 120×40, 104/103-column boundary, 100×36, 80×24, 40×20, and short-wide 100×24. Review rendered snapshots for alignment and legibility.
5. **Roll out:** migrate the other screens one at a time to the catalog; address transcript caching separately.

Completion requires all 35 reports reachable by keyboard and mouse; no clipped numeric columns or unreachable controls; scrolling past 64 records; selections stable through refresh/resize; Ctrl+K working in Profile; filter/range changes reflected in totals; zero/missing/stale/small-sample fixtures; and count/stack totals reconciling with source metrics.

Run repository checks in the existing order: `cargo fmt --all --check`, then `cargo clippy --workspace --all-targets --locked --no-default-features -- -D warnings`, then `cargo test --workspace --locked --no-default-features`, with ARGOS_EMBED unset. Follow the current OpenCode ownership workflow where applicable; update graphify after code changes.

**Audit verification:** source and Codex snapshots were inspected; Argos runtime was not executed because Cargo is unavailable here.

**Pinned Codex references:** [card grid](https://github.com/openai/codex/blob/c3d3b142d10f4316b46e35aad7e5317e7e506cb7/codex-rs/tui/src/analytics/dashboard.rs), [plot pipeline](https://github.com/openai/codex/blob/c3d3b142d10f4316b46e35aad7e5317e7e506cb7/codex-rs/tui/src/analytics/plot.rs), [snapshot tests](https://github.com/openai/codex/tree/c3d3b142d10f4316b46e35aad7e5317e7e506cb7/codex-rs/tui/src/analytics/snapshots).
