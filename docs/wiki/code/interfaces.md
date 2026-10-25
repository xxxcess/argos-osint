# CLI and TUI map

[Code route](index.md) · [Usage](../concepts/usage.md)

`@tag:tui-components`

| Layer | Source | Contract |
| --- | --- | --- |
| CLI | [cli.rs](../../../crates/argos-osint-bin/src/cli.rs) | [Commands](../concepts/usage.md) |
| App state and actions | [tui/app.rs](../../../crates/argos-osint-bin/src/tui/app.rs), [commands.rs](../../../crates/argos-osint-bin/src/tui/commands.rs) | [Design](tui-design-spec.md) |
| Rendering | [tui/ui.rs](../../../crates/argos-osint-bin/src/tui/ui.rs) | [Shell figure](../../diagrams/tui-shell.html) |
| Shared presentation primitives | [components.rs](../../../crates/argos-osint-bin/src/tui/components.rs) | [Component catalog](tui-components.md) |
| Profile geometry | [profile_layout.rs](../../../crates/argos-osint-bin/src/tui/profile_layout.rs) | [Analytics dashboard](profile-analytics-dashboard.md) |
| Storage reads | [db_reads.rs](../../../crates/argos-osint-bin/src/tui/db_reads.rs) | [Responsiveness](tui-database-responsiveness.md) |

For UI work, load `argos-tui-verify`. Follow [verification](tui-verification.md): deterministic TestBackend captures, actual PNG inspection, and separate action/metric checks. Shared layout must govern rendering, hit-testing and focus order. See [contributor workflow](contributing.md) for isolated builds and worktree constraints.
