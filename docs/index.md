# Fleetworks wiki

Understand the product, then follow its implementation. This is the maintained entry point for humans and agents; canonical concepts, implementation contracts and workflow guides live under `docs/wiki/`.

| Route | Start here | Follow it to |
| --- | --- | --- |
| Start Fleetworks | [Venture and workspace guide](wiki/code/fleetworks.md) | Client commands, local Compose, storage modes and rollout status |
| Explore concepts | [Concepts](wiki/concepts/index.md) | Apps, evidence, model roles, persistence and their diagrams |
| Explore code | [Code](wiki/code/index.md) | Crates, module entry points, contracts and verification |

[![Wiki navigation](diagrams/wiki-navigation.svg)](diagrams/wiki-navigation.html)

## Directory map

| Location | Purpose / next stop |
| --- | --- |
| [README.md](../README.md) | Product overview and quick start |
| [docs/archive/](archive/index.md) | Historical checklists, investigations and verification reports |
| [docs/wiki/](wiki/index.md) | Two routes, searchable tags and wiki maintenance |
| [docs/diagrams/](diagrams.md) | Diagram catalog; HTML pages and matching SVG figures |
| [Core crate](../crates/argos-osint-core) | Domain pipelines and storage; [module map](wiki/code/core.md) |
| [Venture](../apps/venture) and [terminal clients](../apps/fleetworks-cli) | Workspace lifecycle and versioned API clients. [Fleetworks guide](wiki/code/fleetworks.md) |
| [Deployment](../deploy/README.md) | Local Compose, database profiles and the [Atlas overlay](../deploy/atlas-dispatch.md) |
| [API adapter](../crates/argos-osint-api) / [host](../services/api-host) | Versioned HTTP contracts and authenticated service roles. [API contract](wiki/code/intel-web.md) |
| [Web app](../apps/web-desktop) / [shared packages](../packages) | Next server/stateless API relay, React Native primitives and typed Query client |
| [scripts/](../scripts) | Builds, storage tests, releases and verification; [workflow](wiki/code/contributing.md) |
| [.opencode/](../.opencode) | Role/command/skill configuration; [guide](wiki/code/opencode-v2.md) |
| [.agents/skills/](../.agents/skills) | Repository skills for other agent hosts |
| [.github/workflows/](../.github/workflows) | CI and release automation; [CI guide](wiki/code/ci.md) |
| [.planning/](../.planning) | Named task work; [format and retention](wiki/code/planning-artifacts.md) |

Search by [tag](wiki/tags.md) when a concern crosses directories. Follow source links, search symbols/callers in the relevant directories, and verify against tests. Navigation requires no generated code graph.

Before changing code, read [contributor workflow](wiki/code/contributing.md) and [conventions](wiki/code/conventions.md). Maintain this hierarchy through [setup / refresh wiki](wiki/maintenance.md).
