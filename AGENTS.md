# Argos OSINT — Agent orientation

Rust OSINT CLI/TUI: Recon, Intel, Atlas and Brain, backed by SurrealDB.

- Start at [docs/index.md](docs/index.md): choose **concepts** for behavior or **code** for implementation; follow links before broad searches.
- Before edits/checks, read [contributor workflow](docs/wiki/code/contributing.md) and relevant [conventions](docs/wiki/code/conventions.md). Preserve unrelated changes. `.github` changes belong to Code Architect.
- Trace cross-cutting features with [tags](docs/wiki/tags.md): `rg -n '@tag:<slug>' docs crates`. Keep affected routes, tags, contracts and diagrams current in the same change; follow [setup/refresh wiki](docs/wiki/maintenance.md).
- CI/build/test work starts at [CI workflow guide](docs/wiki/code/ci.md) and the affected `.github/workflows/` files; select the relevant Rust/web/policy checks and report local versus hosted evidence. `.github` edits remain Code Architect-owned.
- Multi-phase work uses `.planning/<YYYY-MM-DD-slug>/` and the [planning format](docs/wiki/code/planning-artifacts.md#working-format). Update phase item checkboxes as work finishes, plus evidence and Next Step; do not mark unverified work complete.
- React UI work loads `argos-react-ui` (plus `argos-react-patterns` for components/state/performance); follow [React UI rules](docs/wiki/code/react-ui.md). Use the core TUI theme with minimalist web layouts/components; use shared React Native primitives, keep React Compiler/Hooks lint enabled, and inspect fixture screenshots for visual changes.
- TUI work loads `argos-tui-verify` and follows [design](docs/wiki/code/tui-design-spec.md), [components](docs/wiki/code/tui-components.md) and [verification](docs/wiki/code/tui-verification.md). Profile also uses [analytics](docs/wiki/code/profile-analytics-dashboard.md). Inspect captured PNGs.
- OpenCode roles/skills: [.opencode/AGENTS.md](.opencode/AGENTS.md), [workflow](docs/wiki/code/opencode-v2.md). Scratch belongs only in `.agent-scratch/`, unstaged.
