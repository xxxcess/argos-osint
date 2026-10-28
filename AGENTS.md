# Argos OSINT — Agent orientation

Rust OSINT CLI/TUI: Recon, Intel, Atlas and Brain, backed by SurrealDB.

- Start at [docs/index.md](docs/index.md): choose **concepts** for behavior or **code** for implementation; follow links before broad searches.
- Before edits/checks, read [contributor workflow](docs/wiki/code/contributing.md) and relevant [conventions](docs/wiki/code/conventions.md). Preserve unrelated changes. `.github` changes belong to Code Architect.
- Trace cross-cutting features with [tags](docs/wiki/tags.md): `rg -n '@tag:<slug>' docs crates`. Keep affected routes, tags, contracts and diagrams current in the same change; follow [setup/refresh wiki](docs/wiki/maintenance.md).
- CI/build/test work starts at [CI workflow guide](docs/wiki/code/ci.md) and the affected `.github/workflows/` files; select the relevant Rust/web/policy checks and report local versus hosted evidence. Never run the all-platform or `server` storage matrix from `ci.yml` locally: stabilize the **local host machine platform** (`uname -ms`, `rustc -vV | grep '^host:'`) in **embedded storage mode** (`--storage embedded`, default features `embedded,nlp`) first using targeted crate/test commands (`python3 scripts/agent_cargo.py check|clippy|test --locked -p <crate> [--lib | --bin <bin> | --test <name>]`, and `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 … --test storage_<name>`) before `scripts/local_matrix.sh --storage embedded` ([contributing](docs/wiki/code/contributing.md#targeted-local-rust-builds-and-tests-fast-iteration)), and monitor GitHub Actions checks (`gh pr checks`, `gh run list --branch "$(git branch --show-current)"`, `gh run view <run-id> --log-failed`) when available for issues on other platforms. `.github` edits remain Code Architect-owned.
- Multi-phase work uses `.planning/<YYYY-MM-DD-slug>/` and the [planning format](docs/wiki/code/planning-artifacts.md#working-format). Update phase item checkboxes as work finishes, plus evidence and Next Step; do not mark unverified work complete.
- React UI work loads `argos-react-ui` (plus `argos-react-patterns` for components/state/performance); follow [React UI rules](docs/wiki/code/react-ui.md). web palette follows `theme.rs` with the HUD/glass treatment in react-ui.md while preserving layout semantics and contracts; use shared React Native primitives, keep React Compiler/Hooks lint enabled, and inspect fixture screenshots for visual changes.
- TUI work loads `argos-tui-verify` and follows [design](docs/wiki/code/tui-design-spec.md), [components](docs/wiki/code/tui-components.md) and [verification](docs/wiki/code/tui-verification.md). Profile also uses [analytics](docs/wiki/code/profile-analytics-dashboard.md). Inspect captured PNGs.
- OpenCode & Antigravity CLI roles/skills: [.opencode/AGENTS.md](.opencode/AGENTS.md), [OpenCode workflow](docs/wiki/code/opencode-v2.md), and Antigravity `/teamwork` specs in [.agy/howto.md](.agy/howto.md) ([lite](.agy/lite-team.md), [heavy](.agy/heavy-team.md)). Scratch belongs only in `.agent-scratch/`, unstaged.

## Look & Feel — Argos-inspired (React Native primary)

The Argos OSINT terminal desk is the reference for *feel*: professional, evidence-first, sparse chrome, clear hierarchy, controlled density. Most UI work is React Native; the TUI stays simpler and should not grow more complex. Canonical detail: [Argos look and feel](docs/wiki/code/react-ui.md#argos-look--feel-reference) and [React UI rules](docs/wiki/code/react-ui.md).

### Core principles (apply to RN first)

- **Accent-driven hierarchy**: Use the accent color for titles, primary actions, active navigation, and focus rings. Keep everything else neutral.
- **Thin, quiet separation**: Prefer subtle borders or elevation over heavy chrome. One consistent padding unit (tight, ~8–12px equivalent) around content.
- **Density with intent**: Default list/card spacing is comfortable (visible gap between records). Use compact/zero-gap only for intentional dense surfaces (home launchers, command palettes, headline lists, category bullets).
- **Focus vs selection**: Keep them distinct. Accent/bold or ring = activation/focus. A separate select token = current selection. Never collapse other content when focusing.
- **Scroll, don’t squeeze**: Charts, tables, and long content keep minimum readable sizes and scroll. Ellipsize labels; never wrap critical rows.
- **Evidence over decoration**: Bold for key values/KPIs at normal size. No oversized type, gradients, or ornamental elements unless they carry information.
- **Tokens only**: All colors, spacing, and radii come from the shared theme. Do not invent one-off values.
- **Force-graph topology**: React Native topology graphs use the shared force-graph canvas (`react-force-graph-2d` via `ForceCanvas`). Keep the accessible list twin. Do not add another graph library or a custom layout.

### React Native translation

- Map Argos tokens → RN theme (accent, select, surface, text, muted, border).
- Lists: default item separator or padding; explicit compact variant for dense screens.
- Focus: use accessibility focus + visual ring/accent border.
- Cards/panels: thin border + consistent padding; titles in accent.
- Navigation: accent fill or indicator with high-contrast text.
- Topology: `packages/intel` `ForceCanvas` only. The TUI keeps its existing topology view.

### TUI (Argos) — keep simple

- Preserve existing thin borders, one-cell padding, ACCENT titles, ACCENT focus border + `• focused`, SELECT/TEXT for selected rows.
- Do not add new visual elements, animations, or density modes. Extend shared components only.
- Any TUI change must still pass the existing design contract and visual verification.

Agents: name the screen type and density intent before building UI. Prefer the shared theme and existing components.
