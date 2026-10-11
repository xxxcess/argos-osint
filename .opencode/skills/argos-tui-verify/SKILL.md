---
name: argos-tui-verify
description: Implement and verify Argos terminal components, layout, focus and mouse behavior using deterministic TestBackend cell dumps and screenshot review. Use for TUI changes in this repo.
---

Read `docs/tui-verification.md`, `docs/tui-design-spec.md` and relevant `docs/tui-components.md` entries (paths relative to the repo root). The verification guide is the maintained procedure; `docs/tui-verification-session-2026-10-10.md` records concrete screenshot findings.

Before editing, name components, layout preset, owned files, immutable data sources and acceptance cases. Record affected apps/pages, shared consumers, fixture selectors, viewport/state choices and excluded surfaces with reasons; follow the guide’s scope table. Keep drawing, focus and pointer targets on shared geometry; extend shared components instead of copying screen-specific fixes. Preserve IDs, theme tokens and metric semantics.

Use deterministic fixtures with overflow, missing/low-N values and stale/empty/loading states. Assert geometry, state restoration, stable identities, filters and keyboard/mouse parity. A PNG cannot certify an action or metric cohort.

The parent runs fmt → strict Clippy → offline tests with locked dependencies and no default features. Use `python3 scripts/agent_cargo.py` for Clippy/tests so agent artifacts stay in `target/agents` and user runs retain `target/debug`. Capture applies this isolation automatically. Editors follow their host's formatting-only rules. Then capture real terminal output into a **fresh** run directory:

```sh
.agent-scratch/tui-review-env/bin/python scripts/tui_review.py capture --output .planning/<PLAN_ID>/tui/run-01 \
  --test dump_overhaul_screens --include 'tools-page-*' \
  --size 160x50 --size 80x24 --reason "Tools compact page change; wide and compact layouts"
```

Follow the guide's dependency setup. There is no implicit Profile capture. Choose `--test <fixture-test>` and quoted, repeatable `--include` stem globs, optionally `--size`; use `--all` only for an intentional full sweep. Implement the test’s `ARGOS_SCREEN_DIR` export first if absent. Selection happens before PNG rendering; the underlying fixture test still executes all its assertions. Unmatched selectors fail capture. The tool records provenance and validates cells; review starts pending.

Open emitted PNGs with the host's image viewer and compare the viewport/state contract and available approved references. Record image names, observations, fixes, test results and limitations in the review ledger. If the host cannot view images, report visual verification as pending; readable cells are useful but do not complete screenshot review.

Re-capture after output changes. Focused tests suffice for a subsequent interaction-only change when buffers are unchanged and the full gate passed; record that sequence. Run `graphify update .` after code edits, retain prescribed graph files and remove task-owned scratch after checks. Deliver actual evidence links and distinguish live-provider/default-LanceDB/MiniLM checks from offline checks. Do not infer permissions or change models.


TUI overhaul navigation/component contract: read `docs/tui-design-spec.md` and `docs/tui-components.md`, including Shared navigation and portable configuration. Preserve three Profile pages, independent TabBar focus/activation, nested dataset offsets, background image resources, exact-revision Verify/Save and usize transcript scroll. Scope captures to affected Argos apps, pages, states and viewport thresholds using `docs/tui-verification.md`. Select `--test` and `--include` explicitly; use `--size` for affected thresholds and record `--reason`. Shared components require representative affected consumers; shared shell/theme/global navigation or a full overhaul requires both fixture suites with `--all`. Unrelated apps need no PNGs. Generation does not complete visual review.
