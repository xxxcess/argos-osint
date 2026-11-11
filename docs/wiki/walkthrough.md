# Walk through the repository

[Repository map](../index.md) · [Wiki routes](index.md)

Follow these routes from setup to implementation.

## What this repository is

Fleetworks collects public-source intelligence and prepares cited Focus Briefs. Venture starts the services behind the Intel web desktop and six terminal clients. They share an authenticated API and SurrealDB.

Start with the [README](../../README.md), [concepts](concepts/index.md), and [glossary](concepts/glossary.md). Follow the [code map](code/index.md) and [diagrams](../diagrams.md) for implementation.

## Run a workspace

Install Rust 1.95.0, Docker, and Compose v2.20+. The [quick start](../../README.md#quick-start) runs a Compose disk stack. Local development typically uses `venture up --dev --storage local-net` with existing SurrealDB and RabbitMQ. [Workspace setup](code/fleetworks.md) · [Deployment settings](../../deploy/README.md)

`venture web` starts services and opens Jobs and Logs. `venture client <app>` launches a client. `venture stop` stops owned services and preserves workspace data.

## How a request moves

Clients call the gateway API, which routes retained operations to private services. Workers run durable jobs against SurrealDB. Retained API-key model calls use private execution. [Service boundaries](code/architecture.md#fleetworks-service-boundaries) · [Role configuration](code/fleetworks.md#run-the-network-api-and-workers)

## Product surfaces

| Surface | What it is | Read next |
| --- | --- | --- |
| Intel | Bulletin, Focus Briefs, claim analysis, and staged publication | [Usage](concepts/usage.md#intel) · [Brief chain](../diagrams/intel-brief-chain.html) · [Core](code/core.md#intel) |
| Atlas | Durable collection and six-domain admission | [Usage](concepts/usage.md#atlas) · [Classification](concepts/atlas-classification-and-related-articles.md) · [Taxonomy](concepts/strategic-domains-taxonomy.md) · [Pipeline](../diagrams/atlas-pipeline.html) |
| Brain | Sourced memory, associations, topology, and the seeded actor catalog | [Usage](concepts/usage.md#brain) · [Topology](concepts/topology-mapper.md) · [Actor catalog](code/actor-catalog.md) |
| Jobs and Logs | Durable work and operational events | [Usage](concepts/usage.md#jobs-and-logs) · [Lifecycle](../diagrams/jobs-lifecycle.html) |
| Tools, Models, and Profile | Catalog calls, role defaults, workspace health, and portable configuration | [Usage](concepts/usage.md#profile) · [Providers](concepts/providers.md) · [Model roles](../diagrams/model-roles.html) |

The [CLI reference](../../apps/fleetworks-cli/README.md) is the command list. [Interfaces](code/interfaces.md) ties the clients to their verification.

## Persistence

Services share the selected SurrealDB endpoint. Venture can manage the database or connect to an existing server. Compatibility tests retain direct embedded-engine access.

[Storage map](code/storage.md) · [Schema authority](code/storage.md#schema-and-runtime-authority) · [Persistence diagram](../diagrams/persistence.html) · [Recall](code/storage.md#recall-and-source-lifecycle)

## Models, classification, and local text

[Providers](concepts/providers.md) explains route selection, pacing, and where credentials stay. [Request scheduling](../diagrams/model-request-scheduling.html) shows the shared tickets and cooldowns. [Decision contracts](code/jev-decision-gates.md) compile to Jev-native questions or schema-constrained prompts.

Atlas admission uses the [classifier](code/classifier.md): a pinned Rust-BERT worker for inference, and a separately managed Natural training run. [Local NLP](concepts/nlp-preprocessing.md) is Compromise running inside the service, with no Node process at request time. The [NLP figure](../diagrams/nlp-preprocessing.svg) is the data flow.

## Intel web

The browser app is React Native Web, served by Next from [apps/web-desktop](../../apps/web-desktop). It relays to the API on the same origin. Shared screens live in [packages/intel](../../packages/intel), tokens and primitives in [packages/ui](../../packages/ui), and the typed Query client in [packages/api-client](../../packages/api-client).

Start with the [web contract](code/intel-web.md), then the [UI rules](code/react-ui.md) and [browser checks](code/react-ui-testing.md). The [request figure](../diagrams/intel-web.html) shows the relay and readiness boundaries.

## Terminal clients and the retained desk

Venture launches Atlas, Brain, Jobs, Tools, Models, and Profile as HTTP clients. The older Ratatui desk remains for compatibility. [CLI reference](../../apps/fleetworks-cli/README.md) · [Compatibility TUI design](code/tui-design-spec.md) · [TUI verification](code/tui-verification.md)

## Where the code lives

Use the [repository map](../index.md#directory-map) for the directory index.

| Path | Role |
| --- | --- |
| [apps/venture](../../apps/venture) | Workspace launcher |
| [apps/fleetworks-cli](../../apps/fleetworks-cli) | Six network clients |
| [apps/web-desktop](../../apps/web-desktop) | Intel web server and relay |
| [packages](../../packages) | Shared React Native UI, Intel screens, and API client |
| [crates/argos-osint-core](../../crates/argos-osint-core) | Domain workflows and storage |
| [crates/argos-osint-api](../../crates/argos-osint-api) | Versioned HTTP contracts |
| [services/api-host](../../services/api-host) | Authenticated host and private roles |
| [crates/fleetworks-broker](../../crates/fleetworks-broker) | RabbitMQ delivery for Atlas |
| [deploy](../../deploy) | Compose files, images, and smoke scripts |
| [scripts](../../scripts) | Build, storage, release, and CI planning tools |
| [tools/atlas-bert](../../tools/atlas-bert) | Standalone Rust-BERT worker |
| [.github/workflows](../../.github/workflows) | Hosted checks and releases |

The [core map](code/core.md) is the module index inside the domain crate.

## Change, check, and release

Read [contributing](code/contributing.md) and [conventions](code/conventions.md) before editing. [CI scopes](code/ci.md#fleetworksyml--scoped-branch-checks) select checks by changed paths. Embedded compatibility runs on `main` or by dispatch. [Versioning](code/versioning.md) defines the shared workspace version, commits, and releases. Agents follow [execution policy](code/agent-workflow.md).

## Docs, tags, and history

[Tags](tags.md) connect a concern across pages and source comments. [Wiki maintenance](maintenance.md) is how to add or refresh a page without creating a second explanation. Dated investigations stay in the [archive](../archive/index.md).
