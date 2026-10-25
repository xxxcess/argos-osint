# TUI implementation and verification

[Code route](index.md) · [Wiki](../../index.md)

This is the repo standard for agents changing terminal presentation or interaction. Read the [design contract](tui-design-spec.md) and affected [component APIs](tui-components.md) before editing. The [Profile implementation session](../../archive/tui-verification-session-2026-10-10.md) records how this procedure was established, including actual screenshots and defects found. The reusable skill is [argos-tui-verify](../../../.opencode/skills/argos-tui-verify/SKILL.md).

## Plan the contract

Start with the [interface map](interfaces.md), then inspect affected components and callers with scoped searches. Record a named plan under `.planning/<YYYY-MM-DD-slug>/`; one owner maintains phases and the next step. Preserve existing working-tree changes.

Identify components, layout preset, exact owned files, read-only references and acceptance checks before editing. For Profile, use AnalyticsCard, TimePlot, DetailTable, Meter and ScrollPane through `profile_components`, and Dashboard/Report geometry from `profile_layout::LayoutResult`. Other screens use their catalogued presets. Rectangles drive drawing, logical focus and pointer targets together. Keep selection keyed by identity rather than display position.

State the data contract: cohort, denominator, unit, filter scope, rollup coverage, live versus historical facts, missing values and sample threshold. Unknown is not zero. Never add store/network access to draw; render immutable snapshots. Extend shared components when behavior belongs to multiple screens, then update the catalog.

For delegated work, assign disjoint file sets. Child prompts carry components/preset, owned files, read-only references, shared API contracts, exact viewport/data/interaction cases and the parent's validation responsibility. OpenCode editors format their Rust files only. Reviewers receive actual artifacts and completed results, not just acceptance instructions.

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

Include directly affected overlays, empty/loading/stale/error and overflow states. Do not capture unrelated apps merely because a fixture emits them. Keep the full offline test gate for Rust changes; screenshot scope does not narrow test assertions. Broaden coverage when shared consumers or responsive behavior cannot be bounded.

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

## Run the check gate

The parent runs these in order with `ARGOS_EMBED` unset:

```sh
env -u ARGOS_EMBED cargo fmt --all --check
env -u ARGOS_EMBED python3 scripts/agent_cargo.py clippy --workspace --all-targets --locked --no-default-features -- -D warnings
env -u ARGOS_EMBED ARGOS_HOME=/private/tmp/argos-tui-check python3 scripts/agent_cargo.py test --workspace --locked --no-default-features
```

The wrapper and fixture capture reserve `target/agents` for agent artifacts; the user’s normal `cargo run` continues using `target/debug`. Capture manifests record `CARGO_TARGET_DIR` and the explicit Cargo target argument. Do not override the target directory or delete a lock file. See [agent Cargo isolation](contributing.md#build-and-test).

Use a task-specific temporary state path (Linux may use `/tmp`) rather than `~/.argos`. OpenCode shell calls use `timeout: 600000`. `env -u` expresses the unset requirement on macOS/Linux; do not substitute `ARGOS_EMBED=0` when reporting the offline gate. If the sandbox blocks local mock sockets, request host-required escalation and rerun affected checks. A permission failure does not prove a code regression. Ignored MiniLM, default-feature LanceDB and live-provider checks are separate results.

Fix failures without weakening truthful assertions; rerun affected tests and the required gate. After a passing full gate, a final interaction-only edit may use focused follow-up plus fmt/Clippy if buffers are unchanged. Record the sequence instead of asserting an earlier suite ran on later source.

## Capture actual cells

Python 3.10+ and Pillow are needed for PNGs. Install dependencies in task-owned scratch:

```sh
uv venv .agent-scratch/tui-review-env
uv pip install --python .agent-scratch/tui-review-env/bin/python Pillow
.agent-scratch/tui-review-env/bin/python scripts/tui_review.py capture \
  --output .planning/<PLAN_ID>/tui/run-01 --test dump_overhaul_screens \
  --include 'tools-page-*' --size 160x50 --size 80x24 \
  --reason "Tools compact page change; wide and compact layouts"
```

Substitute the selected plan name for `<PLAN_ID>`. Dependency downloads may require host approval. DejaVu Sans Mono on Linux or Courier New on macOS is required; missing fonts are reported. No system Python mutation or Cargo dependency is needed.

The capture tool runs the selected binary test with locked dependencies, no default features, `ARGOS_EMBED` removed and temporary `ARGOS_HOME`. It requires a new directory under `.planning/`, preventing stale dumps from appearing fresh. It validates dimensions/cell fields, renders PNGs and writes a gallery/manifest with command, result, source fingerprint, environment, renderer/font versions and artifact hashes. Review remains **pending**. It does not run the full workspace gate or certify actions/metrics. Incomplete runs remain visible; failed diagnostic logs stay in `.agent-scratch/`. Successful capture removes its log and temporary state.

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
| Intel article imagery | `dump_overhaul_screens` | `intel-image-*`, `intel-brief-image-*` |
| Recon transcript/history | `dump_overhaul_screens` | `recon-transcript-*`, `recon-history-*` |
| Profile Configs | `dump_overhaul_screens` | `configs-draft-*`, `configs-error-*` |
| Shared overlays | `dump_overhaul_screens` | `palette-*`, `help-*`, `record-*`, `overlay-*` |
| Profile analytics | `analytics_viewports_and_data_states` | `profile-recon-*`, `profile-report-*`, `profile-small-sample-*` |
| Profile data states/System | `analytics_viewports_and_data_states` | `profile-empty-*`, `profile-stale-*`, `profile-loading-*`, `profile-system-*` |

For generic app entry views (`app-<index>-*`), resolve the index from `ModuleId` rather than guessing. The selected screenshots retain the real application shell. Filters never crop the terminal buffer.

For an intentional global sweep, run each fixture suite separately with `--all --reason "Shared shell change; all consumers"` and no include/size restriction. A full sweep is also available on request.

The Rust fixture still executes and emits its cases, preserving assertions. The tool stages those dumps temporarily, retains/validates/renders only selected ones, and discards unselected temporary cells without generating PNGs. The manifest records selectors, rationale, emitted count and skipped names; the gallery states selected versus emitted counts. This saves PNG generation, storage and visual review, not fixture execution time. For expensive fixture execution, add a dedicated focused exporter rather than weakening existing tests.

The low-level renderer supports existing dumps:

```sh
.agent-scratch/tui-review-env/bin/python scripts/render_tui_cells.py <cells-dir> <screenshots-dir>
```

JSON is `{width, height, cells}`, exactly `height` rows of `width` cells. Each cell has `s` (symbol), `fg`/`bg` (ratatui debug colors), `b` (bold), `u` (underline). Preserve modifiers in new exporters. PNGs reconstruct cell geometry/colors and selected glyphs; font rasterization is approximate. Box/graph symbols have geometric fallbacks. Wide text uses an optional Noto CJK/Hiragino fallback recorded as `wide_font` in the manifest; backgrounds are painted before text so continuation cells do not erase glyphs. These are actual-buffer visualizations, not live terminal photographs. A renderer workaround does not certify every terminal font; use a terminal smoke test when relevant.

## Review, fix and recapture

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

Update manifest review fields only after review, with a ledger link. Re-capture affected cases to a new run after output, fixture, geometry or renderer changes; preserve the recorded scope unless the fix expands its impact. Unaffected screenshots need no regeneration. Prior visual review may carry forward only when regenerated selected cell and PNG hashes match reviewed evidence and the viewport/state contract and renderer are unchanged; link the earlier ledger and record the comparison. Never describe carried evidence as a new image inspection. Label earlier evidence superseded only for changed cases. Do not overwrite reviewed baselines. Screenshots do not replace action or metric tests.

## Finish and retain evidence

Refresh affected wiki routes, tags and diagrams after code edits. Run `git diff --check`; update catalog/docs and plan results. Remove only task-owned scratch after checks; retain unresolved diagnostic logs.

Select tracked planning deliverables with [planning-artifacts.md](planning-artifacts.md).
Curated acceptance/decision records may belong in Git; raw capture runs and session
working memory stay local. Durable records reference maintained evidence or
regeneration commands rather than ignored run paths.

Keep cells, PNGs, manifest and review ledger under the named plan while work/review is active. Generated `tui/` runs are gitignored. At completion, consolidate reproducible steps, useful defects, exact results and limitations into maintained docs; retain a small referenced fixture selection in `docs/screenshots/<session>/` when it demonstrates a distinct contract. Remove superseded/duplicate/negative-check runs and completed task plans once their useful findings are preserved. Do not remove active plans, unresolved evidence, or unrelated project context. Update any pointers before deleting artifacts. Avoid secrets, live state and dependency environments. Link durable evidence and regeneration commands in the final handoff.
