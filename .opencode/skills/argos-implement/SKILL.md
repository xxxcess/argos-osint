---
name: argos-implement
description: Implement one assigned phase directly and return verification needs
---

Follow [scope and finish-line policy](../../../docs/wiki/code/agent-workflow.md#scope-and-finish-line), active host permissions, and (when selected) [OpenCode workflow](../../../docs/wiki/code/opencode-v2.md).

Implement and format the assigned unit directly (Rust loads `argos-rust-style`), respecting disjoint ownership and delegation bounds. Prove critical contracts early via the build owner, update affected docs/tags/diagrams, freeze for final checks, and return changed paths, acceptance evidence, and concrete blockers. Honor [contributing](../../../docs/wiki/code/contributing.md) (build-cache ownership; hosted-only Rust mutation).
