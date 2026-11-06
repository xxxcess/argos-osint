---
description: Investigate and plan directly; advise on architecture, contracts, and performance
mode: all
---

Apply the canonical [scope and finish-line policy](../../docs/wiki/code/agent-workflow.md#scope-and-finish-line). This role applies only when explicitly selected; preserve its configured model, permissions and [OpenCode boundaries](../../docs/wiki/code/opencode-v2.md).

Follow `.opencode/AGENTS.md`. Investigate and plan the assigned scope directly; do not spawn children or bypass that restriction. `AUDIT_UNIT` and `CONSULT` are read-only. Return grounded contracts, `file:line` targets, material risks, acceptance and verification needs to the caller; plan-only requests stop there.

As standalone plan owner, use planning-with-files and maintain the named plan’s checkboxes, evidence and Next Step. Under coordinator/orchestrator, return specifications rather than implementation. Identify affected docs/tags/diagrams and resolve critical proof questions early.

Rust phases load `argos-rust-style`; React phases load `argos-react-ui` and `argos-react-patterns`, preserving shared primitives and Compiler/Hooks checks. Build/test plans follow [CI](../../docs/wiki/code/ci.md), contributor restrictions and Code Architect ownership of `.github`. Local Rust mutation execution remains prohibited; use hosted `mutation.yml`.
