# Explore Fleetworks code

[Wiki](../../index.md) · [Explore concepts](../concepts/index.md)

Start with the [workspace manifest](../../../Cargo.toml). The native macOS app connects to an independently operated gateway; direct Docker Compose operates the backend. The core crate owns domain workflows and the repository. SurrealDB retains domain persistence; [Jobs persistence](jobs-persistence.md) describes the SQLite/RocksDB target and remaining cutover.

| Branch | What to inspect | Next page |
| --- | --- | --- |
| [Native app](../../../apps/fleetworks-desktop) · [Client contracts](../../../crates/fleetworks-contracts) | Six sidebar destinations plus Status/setup, origin-pinned SDK and protected workstation identity | [Connections](native-connections.md) · [Identity](native-workspaces.md) · [Design system](native-design-system.md) |
| [Core](../../../crates/argos-osint-core/src) | Atlas, Intel, Brain, providers and NLP | [Core map](core.md) |
| [HTTP adapter](../../../crates/argos-osint-api) · [Service host](../../../services/api-host) | Versioned API, authenticated gateway, private Brain, readiness and worker roles | [Intel web](intel-web.md) |
| [Compose](../../../deploy) · [Jobs stores](../../../crates/fleetworks-jobs) | Local service graph and embedded durable delivery | [Deployment](../../../deploy/README.md) |
| [Private actor index](../../../services/search-index) · [Source outbox](../../../crates/argos-osint-core/src/storage/actor_search_outbox.rs) | Canonical discovery, transactional intent and reader-visible generations | [Actor search](actor-search.md) |
| [Storage](../../../crates/argos-osint-core/src/storage) | Open/bootstrap, repositories, publication, jobs and recall | [Storage map](storage.md) |
| [Intel UI](../../../packages/intel) · [Web server](../../../apps/web-desktop) | Browser navigation, Query client and same-origin relay | [React UI rules](react-ui.md) |
| [Core tests](../../../crates/argos-osint-core/tests) · [Scripts](../../../scripts) | Offline contracts and verification entry points | [Contributor workflow](contributing.md) |

## Trace a change

1. Find the concept and its [tag](../tags.md), then follow the mapped source and callers.
2. Verify relationships against current source and tests. The maps use stable module paths.
3. Read the relevant [conventions](conventions.md) and plan exact code, doc and diagram scope with the [planning format](planning-artifacts.md#working-format).
4. Update the affected contracts and [refresh](../maintenance.md#refresh-wiki) before completion.

## Implementation and verification

| Topic | Canonical reference |
| --- | --- |
| Pipeline boundaries and data flow | [Architecture](architecture.md) |
| Six-domain classifier | [Classifier](classifier.md) |
| Model and provider scheduling | [Providers](../concepts/providers.md) |
| Rust style | [Repository Rust style](../../../.opencode/skills/argos-rust-style/SKILL.md) · [Conventions](conventions.md#rust-style) |
| React UI and fixture review | [Design rules](react-ui.md) · [Browser checks](react-ui-testing.md) |
| Build tools, outputs and cache paths | [Visual artifact map](build-artifacts.md) |
| Native Runtime captures and inspection | [Selected native UI verification](native-verification.md) |
| Historical terminal evidence | [Retirement disposition](../../archive/cli-retirement.md) |
| Agent coordination | [Execution policy](agent-workflow.md) · [OpenCode](opencode-v2.md) |
| Docs and figures | [Wiki maintenance](../maintenance.md) · [Diagram catalog](../../diagrams.md) |
| Release and CI | [Versioning](versioning.md) · [CI](ci.md) |

Dated evidence lives in the [archive](../../archive/index.md).
