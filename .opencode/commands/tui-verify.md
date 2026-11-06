---
description: Verify terminal layout, components and interactions with actual fixture screenshots
agent: tester
---

Follow the canonical [scope and finish-line policy](../../docs/wiki/code/agent-workflow.md#scope-and-finish-line) and this command’s configured role/permissions.

Verify $ARGUMENTS with `argos-tui-verify` and `docs/wiki/code/tui-verification.md`: inspect affected PNGs and report visual, interaction and metric evidence separately. Reuse unaffected evidence. Preserve the agreed required native embedded gate for Rust changes; honor user exclusions.
