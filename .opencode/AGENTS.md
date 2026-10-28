# Argos — OpenCode quick guide

Follow root [AGENTS.md](../AGENTS.md); user instructions take precedence. Preserve unrelated changes.

- **Navigate:** [wiki](../docs/index.md) → concepts/code → source; use [tags](../docs/wiki/tags.md) and scoped searches to trace callers/contracts.
- **Team:** architect owns scope/review/shared plans; coder integrates disjoint EDIT_UNITs; Scribe updates docs at phase checkpoints; tester verifies the combined tree. Follow [handoffs/worktrees](../docs/wiki/code/opencode-v2.md). Leaf modes never delegate; resume only your own children.
- **CI:** inspect [workflow contracts](../docs/wiki/code/ci.md) and affected `.github/workflows/` before choosing checks; include export/dev/compiler checks for frontend changes and distinguish local from hosted evidence. Code Architect owns workflow edits.
- **Plan:** pin one named task; use the [planning format](../docs/wiki/code/planning-artifacts.md#working-format). Owner updates phase checkboxes as items finish, evidence and Next Step; unchecked work stays explicit.
- **Verify:** read [contributor workflow](../docs/wiki/code/contributing.md) before edits/checks. Never run the all-platform/`server` CI matrix locally: stabilize the host-native (`uname -ms`, `rustc -vV | grep '^host:'`) embedded (`--storage embedded`) build first with isolated Cargo (`python3 scripts/agent_cargo.py … --locked`) and targeted `-p <crate>` / `--lib` / `--test <name>` iteration, then monitor GitHub CI checks (`gh pr checks`, `gh run view --log-failed`) for other platforms if available. Edit-only worktrees and retry limits apply. TUI work loads argos-tui-verify and inspects PNGs.
- **Rust:** any edit or review under `crates/` loads `argos-rust-style`. Follow [conventions](../docs/wiki/code/conventions.md#async-runtime-and-process-safety) for Tokio; do not apply the guide's Chromium or Fuchsia layout and runtime bans.
- **React UI:** load `argos-react-ui` for visual/interaction work and `argos-react-patterns` for components/state/performance; follow [UI rules](../docs/wiki/code/react-ui.md) and the [look and feel](../docs/wiki/code/react-ui.md#argos-look--feel-reference). Design components creatively inside the Argos theme. Keep React Compiler and Hooks lint enabled. `/react-ui-review` selects tester; inspect fixture PNGs and report gaps.
- **Maintain:** [setup/refresh wiki](../docs/wiki/maintenance.md) updates affected routes, tags, contracts and diagrams in the same change. Report actual validation. Scratch stays in `.agent-scratch/`; commits require authorization and exact staging.

## Design Contract (React Native first, TUI secondary)

- Primary target is React Native. Run the design process in [react-ui.md](../docs/wiki/code/react-ui.md#argos-look--feel-reference). Stay on the Argos theme; component design may be original inside it.
- TUI changes are secondary and must stay simpler: no new chrome, no extra density modes, no decorative elements.
- Always use theme tokens. Accent = titles / focus / primary actions. Select = current selection (distinct from focus).
- Default density is spaced; compact only when the surface is explicitly a launcher, palette, or headline list.
- Focus must never hide other panels or content.
- React Native topology graphs use the force-graph canvas (`ForceCanvas` / `react-force-graph-2d`) plus its accessible list twin. Do not introduce a second graph renderer. The TUI keeps its existing topology view.
- For any visual change, state: component type, density (default or compact), and which tokens are used.
