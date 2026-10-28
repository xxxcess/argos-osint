# Argos OSINT — Agent orientation

Rust OSINT CLI/TUI: Recon, Intel, Atlas and Brain, backed by SurrealDB.

- Start at [docs/index.md](docs/index.md): choose **concepts** for behavior or **code** for implementation; follow links before broad searches.
- Before edits/checks, read [contributor workflow](docs/wiki/code/contributing.md) and relevant [conventions](docs/wiki/code/conventions.md). Preserve unrelated changes. `.github` changes belong to Code Architect.
- Trace cross-cutting features with [tags](docs/wiki/tags.md): `rg -n '@tag:<slug>' docs crates`. Keep affected routes, tags, contracts and diagrams current in the same change; follow [setup/refresh wiki](docs/wiki/maintenance.md).
- CI/build/test work starts at [CI workflow guide](docs/wiki/code/ci.md) and the affected `.github/workflows/` files; select the relevant Rust/web/policy checks and report local versus hosted evidence. Never run the all-platform or `server` storage matrix from `ci.yml` locally: stabilize the **local host machine platform** (`uname -ms`, `rustc -vV | grep '^host:'`) in **embedded storage mode** (`--storage embedded`, default features `embedded,nlp`) first using targeted crate/test commands (`python3 scripts/agent_cargo.py check|clippy|test --locked -p <crate> [--lib | --bin <bin> | --test <name>]`, and `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 … --test storage_<name>`) before `scripts/local_matrix.sh --storage embedded` ([contributing](docs/wiki/code/contributing.md#targeted-local-rust-builds-and-tests-fast-iteration)), and monitor GitHub Actions checks (`gh pr checks`, `gh run list --branch "$(git branch --show-current)"`, `gh run view <run-id> --log-failed`) when available for issues on other platforms. `.github` edits remain Code Architect-owned.
- Multi-phase work uses `.planning/<YYYY-MM-DD-slug>/` and the [planning format](docs/wiki/code/planning-artifacts.md#working-format). Update phase item checkboxes as work finishes, plus evidence and Next Step; do not mark unverified work complete.
- React UI work loads `argos-react-ui` (plus `argos-react-patterns` for components/state/performance); follow [React UI rules](docs/wiki/code/react-ui.md) and the [Argos look and feel](docs/wiki/code/react-ui.md#argos-look--feel-reference). Stay on the terminal-desk theme and design components creatively inside it. Use shared React Native primitives, keep React Compiler/Hooks lint enabled, and inspect fixture screenshots for visual changes.
- Rust work under `crates/` loads `argos-rust-style` (Google's Rust Style Guide). Argos async rules and the existing crate layout win where that guide's Chromium or Fuchsia notes conflict. `cargo fmt` remains the formatter.
- TUI work loads `argos-tui-verify` and follows [design](docs/wiki/code/tui-design-spec.md), [components](docs/wiki/code/tui-components.md) and [verification](docs/wiki/code/tui-verification.md). Profile also uses [analytics](docs/wiki/code/profile-analytics-dashboard.md). Inspect captured PNGs.
- OpenCode & Antigravity CLI roles/skills: [.opencode/AGENTS.md](.opencode/AGENTS.md), [OpenCode workflow](docs/wiki/code/opencode-v2.md), and Antigravity `/teamwork` specs in [.agy/howto.md](.agy/howto.md) ([lite](.agy/lite-team.md), [heavy](.agy/heavy-team.md)). Scratch belongs only in `.agent-scratch/`, unstaged.

## Look & Feel — Argos (React Native primary)

Theme is the evidence-first terminal desk plus HUD/glass. Design components creatively inside it: new layouts and treatments are welcome when they use theme tokens, type, accent, and density. Do not leave the theme. Process: [Argos look and feel](docs/wiki/code/react-ui.md#argos-look--feel-reference).

- Name screen type and density before building. Default spacing; compact only for launchers, palettes, headlines, and category bullets.
- Mono for telemetry/nav, body for prose. One spacing unit. Neutrals plus accent for titles, primary actions, and focus. Select stays a separate token.
- Extend shared components, or add one when the screen needs it. States and contrast are part of the design.
- Topology uses `ForceCanvas` (`react-force-graph-2d`) plus its list twin.
- TUI stays simpler: existing thin borders, ACCENT focus, SELECT for selection. No new chrome or density modes.
