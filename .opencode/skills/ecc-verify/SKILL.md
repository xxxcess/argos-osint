---
name: ecc-verify
description: Verify Argos changes in the repository CI order.
---

Follow [scope and finish-line policy](../../../docs/wiki/code/agent-workflow.md#scope-and-finish-line), [argos-testing](../argos-testing/SKILL.md), and [contributing](../../../docs/wiki/code/contributing.md).

Confirm scope unless already authorized; skip Rust rebuilds for docs/config-only changes. Use the combined integration tree, Cargo wrapper, host-native embedded storage locally, hosted-only Rust mutation, and cache ownership. Prove critical contracts early before frozen final checks; rerun only evidence invalidated by a demonstrated blocker. For Rune/React UI changes, load the matching skill and inspect screenshots. Report commands, exits, executed/skipped counts, and unmet acceptance; pending or skipped checks are not passes.
