# Argos OSINT — Agent orientation

Rust OSINT CLI/TUI: Recon, Intel, Atlas and Brain, backed by SurrealDB.

- Start at [docs/index.md](docs/index.md): choose **concepts** for behavior or **code** for implementation; follow links before broad searches.
- Before edits/checks, read [contributor workflow](docs/wiki/code/contributing.md) and relevant [conventions](docs/wiki/code/conventions.md). Preserve unrelated changes. `.github` changes belong to Code Architect.
- **Testing:** before writing or running tests, read [Argos testing guidelines](.opencode/skills/argos-testing/SKILL.md). Tests must prove behavior, remain isolated/offline, and report skips honestly. Follow the Rust gate and feature-mode restrictions in the contributor workflow.
- Trace cross-cutting features with [tags](docs/wiki/tags.md): `rg -n '@tag:<slug>' docs crates`. Keep affected routes, tags, contracts and diagrams current in the same change; follow [setup/refresh wiki](docs/wiki/maintenance.md).
- Multi-phase work uses `.planning/<YYYY-MM-DD-slug>/` and the [planning format](docs/wiki/code/planning-artifacts.md#working-format). Update phase item checkboxes as work finishes, plus evidence and Next Step; do not mark unverified work complete.
- TUI work loads `argos-tui-verify` and follows [design](docs/wiki/code/tui-design-spec.md), [components](docs/wiki/code/tui-components.md) and [verification](docs/wiki/code/tui-verification.md). Profile also uses [analytics](docs/wiki/code/profile-analytics-dashboard.md). Inspect captured PNGs.
- OpenCode roles/skills: [.opencode/AGENTS.md](.opencode/AGENTS.md), [workflow](docs/wiki/code/opencode-v2.md). Scratch belongs only in `.agent-scratch/`, unstaged.
