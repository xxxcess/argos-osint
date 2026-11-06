---
name: ecc-review
description: Review Argos changes for concrete regressions.
---

Follow the canonical [scope and finish-line policy](../../../docs/wiki/code/agent-workflow.md#scope-and-finish-line) and the active host’s permissions; OpenCode roles apply only when selected.

Review the requested diff and its affected contracts/callers. Report demonstrated acceptance blockers by severity with `file:line` evidence and a minimal correction; otherwise report no findings and relevant limitations. Defer unrelated cleanup and speculative audits.

Use applicable Rust, React or TUI skills. Check affected docs/diagrams under [conventions](../../../docs/wiki/code/conventions.md). The assigned build owner runs verification. Reuse unaffected evidence; review only the changed correction after a blocker is fixed, without an automatic final rereview cycle.
