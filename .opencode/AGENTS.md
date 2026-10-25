# Argos — OpenCode quick guide

Follow root [AGENTS.md](../AGENTS.md); user instructions take precedence. Preserve unrelated changes.

- **Navigate:** [wiki](../docs/index.md) → concepts/code → source; use [tags](../docs/wiki/tags.md) and scoped searches to trace callers/contracts.
- **Team:** architect owns scope/review/shared plans; coder integrates disjoint EDIT_UNITs; tester verifies the combined tree. Follow [handoffs/worktrees](../docs/wiki/code/opencode-v2.md). Leaf modes never delegate; resume only your own children.
- **Plan:** pin one named task; use the [planning format](../docs/wiki/code/planning-artifacts.md#working-format). Owner updates phase checkboxes as items finish, evidence and Next Step; unchecked work stays explicit.
- **Verify:** read [contributor workflow](../docs/wiki/code/contributing.md) before edits/checks. Isolated Cargo, embedded-only local gate, edit-only worktrees and retry limits apply. TUI work loads argos-tui-verify and inspects PNGs.
- **Maintain:** [setup/refresh wiki](../docs/wiki/maintenance.md) updates affected routes, tags, contracts and diagrams in the same change. Report actual validation. Scratch stays in `.agent-scratch/`; commits require authorization and exact staging.
