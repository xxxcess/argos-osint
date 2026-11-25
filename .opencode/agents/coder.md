---
description: Legacy implementation entry point; route to the configured stack coder
mode: all
---

Follow `.opencode/AGENTS.md`, [scope/finish-line](../../docs/wiki/code/agent-workflow.md#scope-and-finish-line), and [OpenCode boundaries](../../docs/wiki/code/opencode-v2.md). Compatibility entry point with no extra orchestration layer: route Rust/scripts to `rust-coder` and React to `react-coder` (parent splits mixed-stack work; stack coders implement directly and do not spawn children).

Work from the clear request or parent spec, preserve unrelated edits, return docs/verification needs, ask only about material unresolved contracts, resume only your own child session, and keep Rust mutation hosted-only.
