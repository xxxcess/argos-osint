---
name: ecc-review
description: Review Argos changes for concrete regressions.
---

Follow [scope and finish-line policy](../../../docs/wiki/code/agent-workflow.md#scope-and-finish-line) and active host permissions.

Review only the requested diff and affected contracts/callers (applying relevant Rust/React/Rune skills and [conventions](../../../docs/wiki/code/conventions.md) for docs/diagrams). Report demonstrated acceptance blockers by severity with `file:line` evidence and minimal fixes; otherwise report no findings and limitations. Defer unrelated cleanup; the build owner runs verification; re-review only blocker fixes without automatic final rereview cycles.
