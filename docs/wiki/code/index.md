# Explore Fleetworks code

[Wiki](../../index.md) · [Explore concepts](../concepts/index.md)

Start with the [workspace manifest](../../../Cargo.toml). The native macOS and Linux x86_64 Rune app starts an Embedded companion or connects to an independently operated Distributed gateway; direct Docker Compose operates the Distributed backend. The core crate owns domain workflows and the repository. SurrealDB retains domain persistence; [Jobs persistence](jobs-persistence.md) describes the SQLite/RocksDB target and remaining cutover.

| Branch | What to inspect | Next page |
| --- | --- | --- |
| [Native app](../../../apps/fleetworks-desktop) · [Client contracts](../../../crates/fleetworks-contracts) · [Workstation library](../../../crates/fleetworks-workstation) | Six sidebar destinations plus Status/Settings/Inbox/Setup, origin-pinned SDK and protected workstation identity | [Connections](native-connections.md) · [Identity](native-workspaces.md) · [Design system](native-design-system.md) · [Assistant chat contract](assistant-chat-contract.md) |
| [Core](../../../crates/argos-osint-core/src) | Atlas, Intel, Brain, providers and NLP | [Core map](core.md) |
| [HTTP adapter](../../../crates/argos-osint-api) · [Service host](../../../services/api-host) | Versioned API, authenticated gateway, private Brain, readiness, Embedded `local-ipc` and worker roles | [Intel web](intel-web.md) |
| [Compose](../../../deploy) · [Jobs stores](../../../crates/fleetworks-jobs) · [Broker crate](../../../crates/fleetworks-broker) | Local service graph, embedded durable delivery and standalone RabbitMQ (`lapin`) transport | [Deployment](../../../deploy/README.md) |
| [Assistant BERT worker](../../../tools/assistant-bert) | Standalone Embedded Inbox QA Rust-BERT worker (`fleetworks-assistant-bert-listener`) | [Inbox QA V2](assistant-chat-contract.md#inbox-qa-v2) · [Pack setup](../../../deploy/inbox-qa.md) |
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
| Approved theme and live-state target | [Glass Canvas reproduction](../../design/matte-canvas/README.md) · [Implementation handoff](https://github.com/xxxcess/argos-osint/blob/15577471cf1e6d0472e918b96ff6868f4834fccf/.planning/2026-11-16-matte-canvas/handoff.md) |
| React UI and fixture review | [Design rules](react-ui.md) · [Browser checks](react-ui-testing.md) |
| Build tools, outputs and cache paths | [Visual artifact map](build-artifacts.md) |
| Native Runtime captures and inspection | [Selected native UI verification](native-verification.md) |
| Historical terminal evidence | [Retirement disposition](../../archive/cli-retirement.md) |
| Agent coordination | [Execution policy](agent-workflow.md) · [OpenCode](opencode-v2.md) |
| Docs and figures | [Wiki maintenance](../maintenance.md) · [Diagram catalog](../../diagrams.md) |
| Performance, security and scalability | [Performance, Security & Scalability](performance-security-scalability.md) ([scorecard](performance-security-scalability.md#baseline-scorecard) · [PR delta rubric](performance-security-scalability.md#scoring-rubric-and-pr-delta-tracking)) |
| Release and CI | [Versioning](versioning.md) · [CI](ci.md) |

Dated evidence lives in the [archive](../../archive/index.md).
