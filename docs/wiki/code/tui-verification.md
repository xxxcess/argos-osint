# TUI implementation and verification

[Code route](index.md) · [Wiki](../../index.md) · [Canonical agent workflow](agent-workflow.md)

This is the repo standard for agents changing terminal presentation or interaction. Read the [design contract](tui-design-spec.md) and affected [component APIs](tui-components.md) before editing. Keep the TUI simple. React Native component creativity belongs in the [Argos look and feel](react-ui.md#argos-look--feel-reference) and does not add terminal chrome. The [Profile implementation session](../../archive/tui-verification-session-2026-10-10.md) records how this procedure was established, including actual screenshots and defects found. The reusable skill is [argos-tui-verify](../../../.opencode/skills/argos-tui-verify/SKILL.md).

## Plan the contract

Start with the [interface map](interfaces.md), then inspect affected components and callers with scoped searches. Scale planning to the task under the [planning format](planning-artifacts.md#working-format); multi-phase work uses a named plan, while small tasks need no separate planning files. Preserve existing working-tree changes and follow the [scope and finish line](agent-workflow.md#scope-and-finish-line).

Identify components, layout preset, exact owned files, read-only references and acceptance checks before editing. For Profile, use AnalyticsCard, TimePlot, DetailTable, Meter and ScrollPane through `profile_components`, and Dashboard/Report geometry from `profile_layout::LayoutResult`. Other screens use their catalogued presets. Rectangles drive drawing, logical focus and pointer targets together. Keep selection keyed by identity rather than display position.

State the data contract: cohort, denominator, unit, filter scope, rollup coverage, live versus historical facts, missing values and sample threshold. Unknown is not zero. Never add store/network access to draw; render immutable snapshots. Extend shared components when behavior belongs to multiple screens, then update the catalog.

For delegated work, assign disjoint file sets. Child prompts carry components/preset, owned files, read-only references, shared API contracts, exact viewport/data/interaction cases and the parent's validation responsibility. When the OpenCode profile is selected, preserve its editor/tester role boundaries; other hosts use their active roles and the shared build owner. Reviewers receive actual artifacts and incremental results, not just acceptance instructions.

## Build fixtures and assertions

Use `ratatui::backend::TestBackend` with deterministic snapshots, timestamps and fake providers. Render the application including its real shell, theme and registry. Examples in `crates/argos-osint-bin/src/tui/app.rs`: `analytics_fixture`, `analytics_viewports_and_data_states`, `dump_cells` and adjacent interaction tests. Draw before clicking so the registry is current; verify resize refreshes geometry.

Choose screenshot scope from the change and its consumers before capturing. Record the affected Argos app, pages/presets, states, fixture test, include patterns, viewport sizes and why other surfaces are excluded. File names alone do not establish scope: an edit in `ui.rs` or `app.rs` may be local, while a shared helper may affect several apps.

| Change | Required visual coverage |
| --- | --- |
| Local label/style/content | Changed page/state at a normal viewport and the smallest or wrapping threshold affected |
| App-specific layout, scrolling or compact page | Changed app's pages/states at affected wide, breakpoint and compact sizes; include below-minimum if its fallback changes |
| Shared component | Each distinct affected consumer/preset at relevant thresholds; expand to all consumers if behavior differs or impact is uncertain |
| Shared Profile geometry or registry | Affected Dashboard/Report pages across the full Profile size matrix; registry/inventory changes cover all 20 IDs and Summary reuse |
| Shared shell, theme, global navigation, full overhaul or uncertain global impact | Both `dump_overhaul_screens` and `analytics_viewports_and_data_states`, all emitted pages/sizes/states |
| Interaction/data-only with unchanged buffers | Focused action/metric tests; no new PNGs required when reviewed evidence still matches output |
| Docs/tooling-only | Validate the workflow/tool; no application-wide screenshot sweep |

Include directly affected overlays, empty/loading/stale/error and overflow states. Do not capture unrelated apps merely because a fixture emits them. Keep the required final native embedded gate for Rust changes; early screenshot scope does not narrow its test assertions. Broaden coverage when shared consumers or responsive behavior cannot be bounded.

Profile's available size matrix is:

| Case | Cells | Purpose |
| --- | --- | --- |
| Wide | 160×50 | Shell rows, simultaneous panels, values and legends |
| Medium | 120×40 | Two-column threshold, constrained labels |
| Narrow | 100×32, 100×24 | Single-column flow and vertical pressure |
| Compact | 80×24 | Report usability and scroll reachability |
| Minimum | 60×18 | Compact KPIs and reachable controls |
| Below minimum | 40×20 | Navigation and explicit too-small notice |

The Profile fixture emits eight page/report cases at seven sizes plus four state cases: 60 dumps. `dump_overhaul_screens` emits 162 dumps for the shell, app pages and overlays. These are fixture inventories, not mandatory PNG counts for every change. Select only affected cases before rendering. Profile's `intel`/`recon`/`tools` names refer to analytics views, not those standalone apps. Configs/System use separate cases. Add a focused exporter when the inventory lacks the changed state.

Include empty, loading, stale/error-with-retained-data, overflow, long Unicode labels/numbers, missing durations, low-N percentiles, unknown/over-limit capacity and signed/fractional series. Keep fixture times inside the chosen period.

Assertions complement image review:

- Rectangles stay within the viewport; heights/columns match the preset.
- Focus reveals content, manual scroll works, panels retain independent offsets, Esc restores state.
- Keyboard and mouse select the same bucket/row/action; identities survive sorting and snapshot reordering.
- Custom dates require timezones/ordered bounds; dimensions affect only applicable sources.
- Owner actions open the exact authoritative thread/article/quota scope.
- Raw/rollup totals deduplicate, unavailable facts remain N/A, gaps remain gaps, low-N p95 is suppressed.

## Fast feedback and the final gate

Follow the [canonical workflow](agent-workflow.md) and [contributor verification procedure](contributing.md#targeted-local-rust-builds-and-tests-fast-iteration). Reuse existing test authorization. One build owner keeps the host/profile/feature graph and `target/agents` cache stable, selects concurrency from measured RAM/pressure, and serializes Cargo. Do not switch between package-only and workspace feature graphs by accident.

Smoke the real affected fixture once its source/inputs are runnable, including its storage/setup prerequisites. Capture a small affected wide/compact selection immediately as a **preview** and inspect the PNGs; this is useful feedback before a full workspace gate. Label the preview in `--reason` and its review ledger. Preserve its actual test result and source/fixture/renderer provenance. A preview does not certify full integration acceptance.

Fix demonstrated failures at the owning layer. A fixture that now requires real classifier admission or actor setup must establish that contract instead of forging ready state or weakening assertions. If a check stalls or repeatedly fails, collect its original bounded error and choose a different diagnostic approach before another expensive run.

Final Rust acceptance still requires `scripts/local_matrix.sh --storage embedded` in the stable integration tree with `ARGOS_EMBED` unset. Follow the contributor workflow for targeted checks and the gate's actual environment rather than duplicating a second manual command matrix here. Local mutation runs, server storage and all-platform matrices remain excluded. Report hosted, ignored MiniLM and live-provider evidence separately.

The repository wrapper/capture tool reserves `target/agents`; the user's `target/debug`, servers and locks remain protected. Use temporary state through the capture tool rather than `~/.argos`. Host-specific permission requirements still apply; a permissions failure is not a code regression.

Track passing source/contract checks and visual evidence independently. Rerun only checks invalidated by relevant source/configuration/fixture/renderer changes; never claim an earlier gate executed later code. Guidance-only edits or unchanged screenshot output do not by themselves require another Rust build. A new arbitrary attempt limit never waives an unmet final gate.

## Capture actual cells

Python 3.10+ and Pillow are needed for PNGs. Install dependencies in task-owned scratch:

```sh
uv venv .agent-scratch/tui-review-env
uv pip install --python .agent-scratch/tui-review-env/bin/python Pillow
.agent-scratch/tui-review-env/bin/python scripts/tui_review.py capture \
  --output .planning/<PLAN_ID>/tui/run-01 --test dump_overhaul_screens \
  --include 'tools-page-*' --size 160x50 --size 80x24 \
  --reason "Preview: Tools compact page change; wide and compact layouts"
```

Substitute the selected plan name for `<PLAN_ID>`. Dependency downloads may require host approval. DejaVu Sans Mono on Linux or Courier New on macOS is required; missing fonts are reported. No system Python mutation or Cargo dependency is needed.

The capture tool runs the selected library test filter with workspace feature unification matching the gate, locked dependencies, the default embedded features, `ARGOS_EMBED` removed and temporary `ARGOS_HOME`. Other library tests are filtered out. It requires a new directory under `.planning/`, preventing stale dumps from appearing fresh. It validates dimensions/cell fields, renders PNGs and writes a gallery/manifest with command, result, source fingerprint, environment, renderer/font versions and artifact hashes. Review remains **pending**. It does not run the full workspace gate or certify actions/metrics. Before the final gate, label the run as a preview; after the gate, retain or refresh only the affected output evidence as described below. Incomplete runs remain visible; failed diagnostic logs stay in `.agent-scratch/`. Successful capture removes its log and temporary state.

For another module, extend a deterministic test to emit the same cell format into `ARGOS_SCREEN_DIR`, then select it:

```sh
.agent-scratch/tui-review-env/bin/python scripts/tui_review.py capture \
  --output .planning/<PLAN_ID>/tui/run-02 --test dump_memory_enter_topology_screens \
  --include 'memory-*' --reason "Brain memory list and Enter opening the topology; normal and narrow states"
python3 scripts/tui_review.py validate .planning/<PLAN_ID>/tui/run-02/cells
```

Only explicitly ignored fixture tests need `--ignored`. A filter matching zero tests or a test without dumps fails capture. Cell validation uses the standard library.

`--test` and either `--include` or `--all` are required; there is no default app. Include patterns match fixture filename stems (without `.json`), use shell quotes, and combine by union. Repeat `--size WIDTHxHEIGHT` to restrict that union by actual cell dimensions; omit it to retain all emitted sizes for the selected pages. Each include pattern and requested size must match a selected dump or capture fails. Inspect the gallery to ensure required page/size combinations exist; selectors do not infer the change's acceptance criteria.

| Surface | Test | Example include patterns |
| --- | --- | --- |
| Tools | `dump_overhaul_screens` | `tools-page-*` |
| Intel text-only article media contract | `intel_article_images_do_not_change_terminal_layout` | `intel-text-bulletin-*`, `intel-text-reader-*` |
| Recon transcript/history | `dump_overhaul_screens` | `recon-transcript-*`, `recon-history-*` |
| Profile Configs | `dump_overhaul_screens` | `configs-draft-*`, `configs-error-*` |
| Shared overlays | `dump_overhaul_screens` | `palette-*`, `help-*`, `record-*`, `overlay-*` |
| Profile analytics | `analytics_viewports_and_data_states` | `profile-recon-*`, `profile-report-*`, `profile-small-sample-*` |
| Profile data states/System | `analytics_viewports_and_data_states` | `profile-empty-*`, `profile-stale-*`, `profile-loading-*`, `profile-system-*` |
| Jobs clear + self-heal | `clearing_jobs_by_status_updates_lists_and_logs_and_drops_stale_reads`, `parked_jobs_tick_self_heals_rows_without_events` | (state assertions; no PNGs required) |
| Logs clear + queue race | `logs_clear_drains_queued_lines_and_late_writes_mark_dirty` | (state assertions; no PNGs required) |
| Brain worker CRUD | `brain_background_delivery_preserves_input_selection_and_last_good_data` (extended with `settle` for async create) | (state assertions; no PNGs required) |
| Profile clear-filters | `profile_clear_filters_schedules_its_own_reload` | (state assertions; no PNGs required) |

For generic app entry views (`app-<index>-*`), resolve the index from `ModuleId` rather than guessing. The selected screenshots retain the real application shell. Filters never crop the terminal buffer.

For an intentional global sweep, run each fixture suite separately with `--all --reason "Shared shell change; all consumers"` and no include/size restriction. A full sweep is also available on request.

The Rust fixture still executes and emits its cases, preserving assertions. The tool stages those dumps temporarily, retains/validates/renders only selected ones, and discards unselected temporary cells without generating PNGs. The manifest records selectors, rationale, emitted count and skipped names; the gallery states selected versus emitted counts. This saves PNG generation, storage and visual review, not fixture execution time. For expensive fixture execution, add a dedicated focused exporter rather than weakening existing tests.

The low-level renderer supports existing dumps:

```sh
.agent-scratch/tui-review-env/bin/python scripts/render_tui_cells.py <cells-dir> <screenshots-dir>
```

JSON is `{width, height, cells}`, exactly `height` rows of `width` cells. Each cell has `s` (symbol), `fg`/`bg` (ratatui debug colors), `b` (bold), `u` (underline). Preserve modifiers in new exporters. PNGs reconstruct cell geometry/colors and selected glyphs; font rasterization is approximate. Box/graph symbols have geometric fallbacks. Wide text uses an optional Noto CJK/Hiragino fallback recorded as `wide_font` in the manifest; backgrounds are painted before text so continuation cells do not erase glyphs. These are actual-buffer visualizations, not live terminal photographs. A renderer workaround does not certify every terminal font; use a terminal smoke test when relevant.

## Review, fix and recapture

Keep an evidence record for each selected case: relevant source/module dependencies, exact fixture test/selectors and inputs, renderer/font identity, cell/PNG hashes, inspection result, and final gate command/source result. The capture manifest already records source and renderer hashes plus fixture selection; add the review observations without rewriting historical provenance. A whole-tree dirty hash is context, not proof that every unrelated documentation change invalidated the screen.


Open PNGs with the host's image viewer. Use cell dumps and geometry assertions to resolve ambiguity. Review changed pages at wide/threshold sizes, then affected compact/minimum/too-small and data states. Inspect shell rows, gutters, borders, palette, selection, numeric clipping, legends/scales, overlap/wrapping, missing-glyph boxes, scroll affordances and control reachability.

Compare supplied approved references and record exact files/differences. If absent, assess the written contract and say comparison was unavailable. Concept art and successful PNG generation are not QA. If the agent cannot view images, leave visual review pending for an image-capable agent or human. Do not silently grant browser/execute access or invent an OpenCode image tool.

Record findings in the run's `README.md` and planning progress:

| Evidence | Record |
| --- | --- |
| Source | Revision, dirty changes, command/environment |
| Visual | PNG names, expectations, observations, pass/defect |
| Interaction | Exact tests/actions and results |
| Metrics | Cohort/rollup/missing/low-N fixtures |
| Fix | Root cause, owner/file, regression and fresh run |
| Limits | Missing references, viewing capability, live/feature checks |

Update manifest review fields only after review, with a ledger link. Re-capture affected cases to a new run after relevant output, fixture, geometry or renderer changes; preserve the recorded scope unless the fix expands its impact. Preview artifacts can support final review when their affected source/fixture/renderer and rendered output remain valid and the required final gate is separately recorded. Unaffected screenshots need no regeneration. Prior visual review may carry forward only when regenerated selected cell and PNG hashes match reviewed evidence and the viewport/state contract and renderer are unchanged; link the earlier ledger and record the comparison. Never describe carried evidence as a new image inspection. Label earlier evidence superseded only for changed cases. Do not overwrite reviewed baselines. Screenshots do not replace action or metric tests.

## Finish and retain evidence

Refresh affected wiki routes, tags and diagrams after code edits. Run `git diff --check`; update catalog/docs and plan results. Remove only task-owned scratch after checks; retain unresolved diagnostic logs.

Select tracked planning deliverables with [planning-artifacts.md](planning-artifacts.md).
Curated acceptance/decision records may belong in Git; raw capture runs and session
working memory stay local. Durable records reference maintained evidence or
regeneration commands rather than ignored run paths.

Keep cells, PNGs, manifest and review ledger under the named plan while work/review is active. Generated `tui/` runs are gitignored. At completion, consolidate reproducible steps, useful defects, exact results and limitations into maintained docs; retain a small referenced fixture selection in `docs/screenshots/<session>/` when it demonstrates a distinct contract. Remove superseded/duplicate/negative-check runs and completed task plans once their useful findings are preserved. Do not remove active plans, unresolved evidence, or unrelated project context. Update any pointers before deleting artifacts. Avoid secrets, live state and dependency environments. Link durable evidence and regeneration commands in the final handoff.
