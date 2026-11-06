---
name: ecc-review
description: Review Argos changes for concrete regressions.
---

Use the canonical [agent workflow](../../../docs/wiki/code/agent-workflow.md) with the active host's roles and existing authorization. OpenCode-only restrictions apply when that profile is explicitly selected.

Inspect the diff, wiki/source relationships, callers, tests and failure paths. Review correctness and performance (complexity, allocation, I/O, locks, queries). Report actionable findings by severity with file:line evidence; otherwise state no findings and remaining risks.

Apply relevant references: argos-tui-verify for TUI evidence; `docs/wiki/code/versioning.md` for commits/releases; `docs/wiki/code/planning-artifacts.md` for retained plans; `docs/wiki/code/conventions.md` for architecture/docs/diagram consistency, HTML/SVG parity and links. Under the OpenCode profile, the assigned tester owns builds; elsewhere follow the active host role and the single integration build owner. Reuse current scoped evidence and request only checks invalidated by the reviewed change.
