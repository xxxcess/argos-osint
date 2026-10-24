---
name: argos-tui-verify
description: Implement and verify Argos terminal components, layout, focus and mouse behavior using deterministic TestBackend cell dumps and screenshot review. Use for TUI changes in this repo.
---

Use `docs/tui-verification.md` as the maintained procedure, with `docs/tui-design-spec.md` and affected `docs/tui-components.md` entries (Profile: `docs/profile-analytics-dashboard.md`).

Before edits, identify components, shared consumers, owned files, data/metric contracts and acceptance. Preserve shared draw/focus/pointer geometry, IDs, theme tokens, navigation/state restoration and keyboard/mouse parity. Fixtures cover affected viewport thresholds, overflow and missing/low-N/stale/empty/loading states.

EDIT_UNIT children edit/format only. Tester runs the embedded gate in the combined integration tree per `.opencode/AGENTS.md`, then captures fresh TestBackend output:

```sh
.agent-scratch/tui-review-env/bin/python scripts/tui_review.py capture --output .planning/<PLAN_ID>/tui/run-01 \
  --test dump_overhaul_screens --include 'tools-page-*' \
  --size 160x50 --size 80x24 --reason "Tools wide/compact layout change"
```

Follow the guide's dependency setup. Choose explicit `--test`, quoted `--include` globs, affected `--size` and `--reason`; implement ARGOS_SCREEN_DIR fixture export if absent. No implicit Profile sweep. Shared shell/theme/global navigation or full overhauls require both fixture suites with `--all`; other shared components need representative consumers. Unmatched selectors fail; selected rendering still runs the fixture's assertions.

Open emitted PNGs and compare contracts/available approved references. Record image paths, observations, fixes and limitations; generation or readable cells alone is not visual review. Verify interactions and metric cohorts with tests separately. Missing image/live evidence stays pending. Re-capture changed output; interaction-only changes may use focused tests when buffers are unchanged and the gate passed. Refresh graphify after code edits and retain artifacts per the guide.
