# TUI implementation and verification

This is the repo standard for agents changing terminal presentation or interaction. Read the [design contract](tui-design-spec.md) and affected [component APIs](tui-components.md) before editing. The [Profile implementation session](tui-verification-session-2026-10-10.md) records how this procedure was established, including actual screenshots and defects found. The reusable skill is [argos-tui-verify](../.opencode/skills/argos-tui-verify/SKILL.md).

## Plan the contract

Start with `graphify query "<affected screen / component / state>"`, then read returned source paths. Record a named plan under `.planning/<YYYY-MM-DD-slug>/`; one owner maintains phases and the next step. Preserve existing working-tree changes.

Identify components, layout preset, exact owned files, read-only references and acceptance checks before editing. For Profile, use AnalyticsCard, TimePlot, DetailTable, Meter and ScrollPane through `profile_components`, and Dashboard/Report geometry from `profile_layout::LayoutResult`. Other screens use their catalogued presets. Rectangles drive drawing, logical focus and pointer targets together. Keep selection keyed by identity rather than display position.

State the data contract: cohort, denominator, unit, filter scope, rollup coverage, live versus historical facts, missing values and sample threshold. Unknown is not zero. Never add store/network access to draw; render immutable snapshots. Extend shared components when behavior belongs to multiple screens, then update the catalog.

For delegated work, assign disjoint file sets. Child prompts carry components/preset, owned files, read-only references, shared API contracts, exact viewport/data/interaction cases and the parent's validation responsibility. OpenCode editors format their Rust files only. Reviewers receive actual artifacts and completed results, not just acceptance instructions.

## Build fixtures and assertions

Use `ratatui::backend::TestBackend` with deterministic snapshots, timestamps and fake providers. Render the application including its real shell, theme and registry. Examples in `crates/argos-osint-bin/src/tui/app.rs`: `analytics_fixture`, `analytics_viewports_and_data_states`, `dump_cells` and adjacent interaction tests. Draw before clicking so the registry is current; verify resize refreshes geometry.

Choose a matrix proportional to the changed surface. Profile's matrix is:

| Case | Cells | Purpose |
| --- | --- | --- |
| Wide | 160×50 | Shell rows, simultaneous panels, values and legends |
| Medium | 120×40 | Two-column threshold, constrained labels |
| Narrow | 100×32, 100×24 | Single-column flow and vertical pressure |
| Compact | 80×24 | Report usability and scroll reachability |
| Minimum | 60×18 | Compact KPIs and reachable controls |
| Below minimum | 40×20 | Navigation and explicit too-small notice |

The Profile fixture emits eight page/report cases at seven sizes plus four state cases: 60 dumps. Do not require all 60 for unrelated small edits; test affected thresholds/states. This matrix does not cover Configs or all other modules. Add their focused cases when affected.

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

The wrapper and fixture capture reserve `target/agents` for agent artifacts; the user’s normal `cargo run` continues using `target/debug`. Capture manifests record `CARGO_TARGET_DIR` and the explicit Cargo target argument. Do not override the target directory or delete a lock file. See [agent Cargo isolation](../AGENTS.md#agent-cargo-isolation).

Use a task-specific temporary state path (Linux may use `/tmp`) rather than `~/.argos`. OpenCode shell calls use `timeout: 600000`. `env -u` expresses the unset requirement on macOS/Linux; do not substitute `ARGOS_EMBED=0` when reporting the offline gate. If the sandbox blocks local mock sockets, request host-required escalation and rerun affected checks. A permission failure does not prove a code regression. Ignored MiniLM, default-feature LanceDB and live-provider checks are separate results.

Fix failures without weakening truthful assertions; rerun affected tests and the required gate. After a passing full gate, a final interaction-only edit may use focused follow-up plus fmt/Clippy if buffers are unchanged. Record the sequence instead of asserting an earlier suite ran on later source.

## Capture actual cells

Python 3.10+ and Pillow are needed for PNGs. Install dependencies in task-owned scratch:

```sh
uv venv .agent-scratch/tui-review-env
uv pip install --python .agent-scratch/tui-review-env/bin/python Pillow
.agent-scratch/tui-review-env/bin/python scripts/tui_review.py capture --output .planning/<PLAN_ID>/tui/run-01
```

Substitute the selected plan name for `<PLAN_ID>`. Dependency downloads may require host approval. DejaVu Sans Mono on Linux or Courier New on macOS is required; missing fonts are reported. No system Python mutation or Cargo dependency is needed.

The capture tool runs the selected binary test with locked dependencies, no default features, `ARGOS_EMBED` removed and temporary `ARGOS_HOME`. It requires a new directory under `.planning/`, preventing stale dumps from appearing fresh. It validates dimensions/cell fields, renders PNGs and writes a gallery/manifest with command, result, source fingerprint, environment, renderer/font versions and artifact hashes. Review remains **pending**. It does not run the full workspace gate or certify actions/metrics. Incomplete runs remain visible; failed diagnostic logs stay in `.agent-scratch/`. Successful capture removes its log and temporary state.

For another module, extend a deterministic test to emit the same cell format into `ARGOS_SCREEN_DIR`, then select it:

```sh
.agent-scratch/tui-review-env/bin/python scripts/tui_review.py capture --output .planning/<PLAN_ID>/tui/run-02 --test dump_phase5b_screens --ignored
python3 scripts/tui_review.py validate .planning/<PLAN_ID>/tui/run-02/cells
```

Only explicitly ignored fixture tests need `--ignored`. A filter matching zero tests or a test without dumps fails capture. Cell validation uses the standard library. The low-level renderer still supports existing dumps:

```sh
.agent-scratch/tui-review-env/bin/python scripts/render_tui_cells.py <cells-dir> <screenshots-dir>
```

JSON is `{width, height, cells}`, exactly `height` rows of `width` cells. Each cell has `s` (symbol), `fg`/`bg` (ratatui debug colors), `b` (bold), `u` (underline). Preserve modifiers in new exporters. PNGs reconstruct cell geometry/colors and selected glyphs; font rasterization is approximate. These are actual-buffer visualizations, not live terminal photographs. A geometric renderer workaround does not certify every terminal font; use a terminal smoke test when relevant.

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

Update manifest review fields only after review, with a ledger link. Re-capture to a new run after output, fixture, geometry or renderer changes; label earlier evidence superseded. Do not overwrite reviewed baselines. Screenshots do not replace action or metric tests.

## Finish and retain evidence

Run `graphify update .` after code edits; inspect warnings. Prescribed tracked graph artifacts are `graph.json`, `manifest.json`, `GRAPH_REPORT.md`. Run `git diff --check`; update catalog/docs and plan results. Remove only task-owned scratch after checks; retain unresolved diagnostic logs.

Keep cells, PNGs, manifest and review ledger under the named plan while work/review is active. Generated `tui/` runs are gitignored. At completion, consolidate reproducible steps, useful defects, exact results and limitations into maintained docs; retain a small referenced fixture selection in `docs/screenshots/<session>/` when it demonstrates a distinct contract. Remove superseded/duplicate/negative-check runs and completed task plans once their useful findings are preserved. Do not remove active plans, unresolved evidence, or unrelated project/GSD context. Update any pointers before deleting artifacts. Avoid secrets, live state and dependency environments. Link durable evidence and regeneration commands in the final handoff.
