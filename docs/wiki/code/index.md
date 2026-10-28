# Explore Argos code

[Wiki](../../index.md) · [Explore concepts](../concepts/index.md)

Start with the [workspace manifest](../../../Cargo.toml) and [workspace diagram](../../diagrams/workspace.html). The core crate owns domain behavior; the binary crate exposes it through CLI/TUI, and the API adapter/host serve the local Intel web client. SurrealDB is the current persistence backend.

| Branch | What to inspect | Next page |
| --- | --- | --- |
| [argos-osint-core](../../../crates/argos-osint-core/src) | Recon, Atlas, Intel, Brain, providers and NLP | [Core map](core.md) |
| [Intel HTTP API / host](../../../crates/argos-osint-api) | Local web adapter and readiness-gated process lifecycle | [Intel web](intel-web.md) |
| [storage](../../../crates/argos-osint-core/src/storage) | Open/bootstrap, repositories, publication, jobs and recall | [Storage map](storage.md) |
| [argos-osint-bin](../../../crates/argos-osint-bin/src) | CLI, state, rendering and shared components | [Interfaces](interfaces.md) |
| [core tests](../../../crates/argos-osint-core/tests) and [scripts](../../../scripts) | Offline contracts and verification entry points | [Contributor workflow](contributing.md) |

## Trace a change

1. Find the concept and its [tag](../tags.md); follow the mapped source and callers.
2. Follow module links and scoped symbol/caller searches; verify relationships against current source and tests.
3. Read relevant tests and [conventions](conventions.md); plan exact code, doc and diagram scope using the [planning format](planning-artifacts.md#working-format).
4. Update affected wiki links/contracts as the implementation changes; [refresh](../maintenance.md#refresh-wiki) before completion.

The maps link stable module paths rather than line numbers. Search symbols in those files for finer navigation. Legacy SQL and migration-era comments still exist: confirm the active feature path in the manifests and storage module before treating them as current contracts.

## Implementation contracts

| Topic | Canonical reference |
| --- | --- |
| Local Intel web/API and stack launcher | [Intel web](intel-web.md) |
| Pipeline boundaries and data flow | [Architecture](architecture.md) |
| Investigation execution and tool decisions | [Harness](investigation-harness.md), [decision gates](jev-decision-gates.md) |
| React UI design, components and review | [React UI rules](react-ui.md), [Argos look and feel](react-ui.md#argos-look--feel-reference) |
| TUI layout and components | [Design contract](tui-design-spec.md), [component catalog](tui-components.md) |
| Profile presentation and data | [Dashboard geometry](profile-analytics-dashboard.md), [metrics and configuration](profile-dashboard-and-search.md) |
| Background reads and responsiveness | [Database responsiveness](tui-database-responsiveness.md) |

## Change and verify the repo

| Task | Canonical procedure |
| --- | --- |
| Build and contribute | [Contributor workflow](contributing.md), [code conventions](conventions.md) |
| Verify TUI behavior | [TUI verification](tui-verification.md) |
| Plan and track implementation | [Planning format and retention](planning-artifacts.md) |
| Use OpenCode roles and handoffs | [OpenCode](opencode-v2.md) |
| Maintain docs | [Wiki setup / refresh](../maintenance.md), [diagram catalog and rules](../../diagrams.md) |
| Release and CI | [Versioning](versioning.md), [CI](ci.md) |

Historical checklists and test reports live in the [archive](../../archive/index.md). Read them for dated evidence, not as current instructions.
