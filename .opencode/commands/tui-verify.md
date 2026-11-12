---
description: Verify terminal layout, components and interactions with actual fixture screenshots
agent: tester
---

Follow the canonical [scope and finish-line policy](../../docs/wiki/code/agent-workflow.md#scope-and-finish-line) and this command’s configured role/permissions.

Verify retained terminal $ARGUMENTS with `argos-tui-verify` and `docs/wiki/code/tui-verification.md`: inspect affected PNGs and report visual, interaction and metric evidence separately. Rune app work uses `fleetworks-rune-verify` and `docs/wiki/code/native-verification.md`. Reuse unaffected evidence. Preserve the agreed embedded gate for affected compatibility/storage changes; honor user exclusions.
