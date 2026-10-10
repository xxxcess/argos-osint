---
name: argos-tui-verify
description: Implement and verify Argos terminal components, layout, focus and mouse behavior using deterministic TestBackend cell dumps and screenshot review. Use for TUI changes in this repo.
---

Read `docs/tui-verification.md`, `docs/tui-design-spec.md` and relevant `docs/tui-components.md` entries (paths relative to the repo root). The verification guide is the maintained procedure; `docs/tui-verification-session-2026-10-10.md` records concrete screenshot findings.

Before editing, name components, layout preset, owned files, immutable data sources and acceptance cases. Keep drawing, focus and pointer targets on shared geometry; extend shared components instead of copying screen-specific fixes. Preserve IDs, theme tokens and metric semantics.

Use deterministic fixtures with overflow, missing/low-N values and stale/empty/loading states. Assert geometry, state restoration, stable identities, filters and keyboard/mouse parity. A PNG cannot certify an action or metric cohort.

The parent runs fmt → strict Clippy → offline tests with locked dependencies and no default features. Editors follow their host's formatting-only rules. Then capture real terminal output into a **fresh** run directory:

```sh
.agent-scratch/tui-review-env/bin/python scripts/tui_review.py capture --output .planning/<PLAN_ID>/tui/run-01
```

Follow the guide's dependency setup. The default fixture covers Profile; use `--test <fixture-test>` for another surface and implement that test's `ARGOS_SCREEN_DIR` export first. The tool records provenance and validates cells; review starts pending.

Open emitted PNGs with the host's image viewer and compare the viewport/state contract and available approved references. Record image names, observations, fixes, test results and limitations in the review ledger. If the host cannot view images, report visual verification as pending; readable cells are useful but do not complete screenshot review.

Re-capture after output changes. Focused tests suffice for a subsequent interaction-only change when buffers are unchanged and the full gate passed; record that sequence. Run `graphify update .` after code edits, retain prescribed graph files and remove task-owned scratch after checks. Deliver actual evidence links and distinguish live-provider/default-LanceDB/MiniLM checks from offline checks. Do not infer permissions or change models.
