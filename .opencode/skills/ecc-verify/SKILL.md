---
name: ecc-verify
description: Verify Argos changes in the repository CI order.
---

Follow the canonical [scope and finish-line policy](../../../docs/wiki/code/agent-workflow.md#scope-and-finish-line) and the active host’s permissions; OpenCode roles apply only when selected.

Follow the selected tester role and [Argos testing guidelines](../argos-testing/SKILL.md). Confirm test scope unless already authorized; use the agreed checks for affected contracts. Documentation/config-only work does not require a Rust rebuild.

Use the stable combined integration tree and [contributor workflow](../../../docs/wiki/code/contributing.md) for required gates, Cargo wrapper, embedded-only local storage, hosted mutation execution and cache ownership. Run early contract proof before the frozen final checks. Rerun only evidence invalidated by a demonstrated acceptance blocker; do not repeat unchanged broad gates.

For Rune/React UI changes use the corresponding skills and inspect affected screenshots. Report actual commands, exits, executed/skipped counts and unmet acceptance; pending or skipped checks are not passes.
