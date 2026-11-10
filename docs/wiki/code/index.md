# Explore Fleetworks code

[Wiki](../../index.md) · [Explore concepts](../concepts/index.md)

Start with the [workspace manifest](../../../Cargo.toml) and [service diagram](../../diagrams/fleetworks.html). Venture manages the stack and launches its clients. The core crate owns domain workflows and the repository. SurrealDB is the persistence backend.

| Branch | What to inspect | Next page |
| --- | --- | --- |
| [Core](../../../crates/argos-osint-core/src) | Atlas, Intel, Brain, providers and NLP | [Core map](core.md) |
| [HTTP adapter](../../../crates/argos-osint-api) · [Service host](../../../services/api-host) | Versioned API, authenticated gateway, private Brain, readiness and worker roles | [Intel web](intel-web.md) |
| [Venture](../../../apps/venture) · [Terminal clients](../../../apps/fleetworks-cli) | Workspace setup, service lifecycle and versioned HTTP clients | [Interfaces](interfaces.md) |
| [Compose](../../../deploy) · [Broker](../../../crates/fleetworks-broker) | Local service graph and Atlas RabbitMQ delivery | [Deployment](../../../deploy/README.md) |
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
| Retained Ratatui source | [Contributor verification](tui-verification.md) |
| Agent coordination | [Execution policy](agent-workflow.md) · [OpenCode](opencode-v2.md) |
| Docs and figures | [Wiki maintenance](../maintenance.md) · [Diagram catalog](../../diagrams.md) |
| Release and CI | [Versioning](versioning.md) · [CI](ci.md) |

Dated evidence lives in the [archive](../../archive/index.md).
