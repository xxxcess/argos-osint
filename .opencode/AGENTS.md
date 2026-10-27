# Argos — OpenCode quick guide

Follow root [AGENTS.md](../AGENTS.md); user instructions take precedence. Preserve unrelated changes.

- **Navigate:** [wiki](../docs/index.md) → concepts/code → source; use [tags](../docs/wiki/tags.md) and scoped searches to trace callers/contracts.
- **Team:** architect owns scope/review/shared plans; coder integrates disjoint EDIT_UNITs; Scribe updates docs at phase checkpoints; tester verifies the combined tree. Follow [handoffs/worktrees](../docs/wiki/code/opencode-v2.md). Leaf modes never delegate; resume only your own children.
- **CI:** inspect [workflow contracts](../docs/wiki/code/ci.md) and affected `.github/workflows/` before choosing checks; include export/dev/compiler checks for frontend changes and distinguish local from hosted evidence. Code Architect owns workflow edits.
- **Plan:** pin one named task; use the [planning format](../docs/wiki/code/planning-artifacts.md#working-format). Owner updates phase checkboxes as items finish, evidence and Next Step; unchecked work stays explicit.
- **Verify:** read [contributor workflow](../docs/wiki/code/contributing.md) before edits/checks. Isolated Cargo, embedded-only local gate, edit-only worktrees and retry limits apply. TUI work loads argos-tui-verify and inspects PNGs.
- **React UI:** load `argos-react-ui` for visual/interaction work and `argos-react-patterns` for components/state/performance; follow [UI rules](../docs/wiki/code/react-ui.md): core TUI theme, minimalist React Native components, React Compiler and Hooks lint enabled. `/react-ui-review` selects tester; inspect fixture PNGs and report gaps.
- **Maintain:** [setup/refresh wiki](../docs/wiki/maintenance.md) updates affected routes, tags, contracts and diagrams in the same change. Report actual validation. Scratch stays in `.agent-scratch/`; commits require authorization and exact staging.
