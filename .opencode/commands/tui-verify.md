---
description: Verify terminal layout, components and interactions with actual fixture screenshots
agent: tester
---

Follow the canonical [agent workflow](../../docs/wiki/code/agent-workflow.md) for execution, evidence reuse and improvement suggestions. Preserve this command's configured OpenCode role when selected; other hosts retain their active instructions.

Verify $ARGUMENTS with argos-tui-verify and `docs/wiki/code/tui-verification.md`: early labelled previews, actual PNG review, separate interaction/metric checks and explicit pending evidence. Reuse inspected output only when relevant source, fixture and renderer provenance remains valid; capture fresh affected cases when that output changes. Final Rust acceptance still requires the native embedded gate.
