---
name: ecc-review
description: Review Argos changes for concrete regressions.
---
1. Inspect the diff and query graphify for affected relationships.
2. Check changed behavior against callers, tests, and failure paths.
3. Report findings in severity order with file and line references. If none, say so and name any unverified risk.

TUI process and artifact retention: `docs/tui-verification.md` / `argos-tui-verify`. The parent owns capture and image/action/metric verification; editors remain formatting-only.
