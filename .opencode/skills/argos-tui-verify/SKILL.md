---
name: argos-tui-verify
description: Implement and verify Argos terminal components, layout, focus and mouse behavior using deterministic TestBackend cell dumps and screenshot review. Use for TUI changes in this repo.
---

Use the canonical [agent workflow](../../../docs/wiki/code/agent-workflow.md) with the active host's roles and existing authorization. OpenCode-only restrictions apply when that profile is explicitly selected.

Use `docs/wiki/code/tui-verification.md` as the maintained procedure, with `docs/wiki/code/tui-design-spec.md` and affected `docs/wiki/code/tui-components.md` entries (Profile: `docs/wiki/code/profile-analytics-dashboard.md`).

Before edits, identify components, shared consumers, owned files, data/metric contracts and acceptance. Preserve shared draw/focus/pointer geometry, IDs, theme tokens, navigation/state restoration and keyboard/mouse parity. Do not add chrome, motion, or density modes. React Native component creativity in the Argos theme does not apply to the TUI. Fixtures cover affected viewport thresholds, overflow and missing/low-N/stale/empty/loading states.

Implement within the active host/profile role. Capture affected TestBackend fixtures early as labelled previews once the focused fixture is runnable; do not defer layout feedback until an expensive full gate. The build owner preserves the gate-compatible feature graph. Final approval still requires the passing native embedded gate and matching output provenance:

```sh
.agent-scratch/tui-review-env/bin/python scripts/tui_review.py capture --output .planning/<PLAN_ID>/tui/run-01 \
  --test dump_overhaul_screens --include 'tools-page-*' \
  --size 160x50 --size 80x24 --reason "Tools wide/compact layout change"
```

Follow the guide's dependency setup. Choose explicit `--test`, quoted `--include` globs, affected `--size` and `--reason`; implement ARGOS_SCREEN_DIR fixture export if absent. No implicit Profile sweep. Shared shell/theme/global navigation or full overhauls require both fixture suites with `--all`; other shared components need representative consumers. Unmatched selectors fail; selected rendering still runs the fixture's assertions.

Open emitted PNGs and compare contracts/available approved references. Record image paths, observations, fixes and limitations; generation or readable cells alone is not visual review. Verify interactions and metric cohorts with tests separately. Missing image/live evidence stays pending. Record source/fixture/renderer hashes and mark pre-gate artifacts as previews. Re-capture only output invalidated by relevant changes; identical inspected cells/PNGs can be carried forward with explicit provenance and final gate evidence. Interaction-only changes may use focused tests when buffers are unchanged and the gate passed. Refresh affected wiki routes/tags/diagrams after code edits and retain artifacts per the guide.

## Build cache hygiene

Follow [cache retention and disk budgets](../../../docs/wiki/code/contributing.md#build-cache-hygiene) before builds, between large checks and at handoff. The build owner proactively removes verified obsolete inactive artifacts, preserves the selected reusable cache and executable, and records free space. Roles without build ownership report cleanup needs to the owner; do not bypass role permissions or use blanket/global cache cleanup.
