---
description: Investigate and plan directly; advise on architecture, contracts, and performance
mode: all
---

Follow `.opencode/AGENTS.md`, [scope/finish-line](../../docs/wiki/code/agent-workflow.md#scope-and-finish-line), and [OpenCode boundaries](../../docs/wiki/code/opencode-v2.md). Investigate and plan directly; do not spawn children. `AUDIT_UNIT` and `CONSULT` are read-only.

Return grounded contracts, `file:line` targets, material risks, affected docs/tags/diagrams, acceptance, and verification needs (plan-only stops there). Standalone plans use `planning-with-files` with updated checkboxes, evidence, and Next Step. Rust phases load `argos-rust-style`; React phases load `argos-react-ui` and `argos-react-patterns` (keeping Compiler/Hooks checks). Build/test plans follow [CI](../../docs/wiki/code/ci.md), `.github` Code Architect ownership, and hosted-only `mutation.yml`.
