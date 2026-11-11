# Walk through the repository

[Repository map](../index.md) · [Wiki routes](index.md)

This is a reading order for the whole checkout. Each section names the canonical page for that area. Follow the link before treating this page as the contract.

## What this repository is

Fleetworks collects public-source intelligence, connects evidence to actors and articles, and prepares cited Focus Briefs. Venture starts the local services. The Intel web desktop and six terminal clients talk to an authenticated API. SurrealDB holds the application records.

Read the [product overview](../../README.md), then the [concept route](concepts/index.md) and the [glossary](concepts/glossary.md). The [code route](code/index.md) is the map from those ideas to crates and tests. The [diagram catalog](../diagrams.md) lists the figures; open the HTML page when you want the larger view.

## Run a workspace

Install Rust 1.95.0, Docker, and Docker Compose v2.20 or newer. The [README quick start](../../README.md#quick-start) builds Venture and its clients and starts the default stack. The [workspace guide](code/fleetworks.md) covers initialization, configuration import, client commands, storage modes, and the current rollout. The [deployment guide](../../deploy/README.md) covers Compose profiles, credentials, and the client and web images. Live Atlas collection uses the [Atlas overlay](../../deploy/atlas-dispatch.md).

From a built checkout, `venture up` starts the API, private services, workers, web relay, and disk-backed SurrealDB. `venture client <app>` opens one of the six clients. `venture web` adds the interactive Jobs and Logs session. `venture stop` keeps the database directory.

## How a request moves

[Service boundaries](code/architecture.md#fleetworks-service-boundaries) name the owner of each process. Venture and the [workspace library](../../crates/fleetworks-workspace) own the workspace and the stack. The [terminal clients](../../apps/fleetworks-cli) and [contracts](../../crates/fleetworks-contracts) speak the versioned HTTP API. The [API adapter](../../crates/argos-osint-api) and [service host](../../services/api-host) authenticate callers, bootstrap the schema, and forward work to private roles. The [core crate](../../crates/argos-osint-core) owns domain workflows, storage, and provider routing.

The public gateway keeps live event streams. Private Brain serves actors, retained briefs, Related articles, and topology. A brief worker claims stages and drains stored publication. Private Tools, Control, and Jobs each have their own bearer. Retained API-key model calls go to the private execution role. Local endpoints and the ChatGPT subscription stay in the calling process. The [Fleetworks guide](code/fleetworks.md#run-the-network-api-and-workers) lists the roles and the environment they need.

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

SurrealDB is the only application database. Venture can use a Compose disk database, a host process in disk or memory mode, or an external disk server. Service processes open that datastore. New homes use server storage. The embedded engine remains for the compatibility suite, and an existing embedded home can be kept.

Read the [storage map](code/storage.md), including [network storage](code/storage.md#fleetworks-network-storage) and [schema authority](code/storage.md#schema-and-runtime-authority). The [persistence figure](../diagrams/persistence.html) shows the selections. [Recall](code/storage.md#recall-and-source-lifecycle) covers fenced vector search and lexical fallback.

## Models, classification, and local text

[Providers](concepts/providers.md) explains route selection, pacing, and where credentials stay. [Request scheduling](../diagrams/model-request-scheduling.html) shows the shared tickets and cooldowns. [Decision contracts](code/jev-decision-gates.md) compile to Jev-native questions or schema-constrained prompts.

Atlas admission uses the [classifier](code/classifier.md): a pinned Rust-BERT worker for inference, and a separately managed Natural training run. [Local NLP](concepts/nlp-preprocessing.md) is Compromise running inside the service, with no Node process at request time. The [NLP figure](../diagrams/nlp-preprocessing.svg) is the data flow.

## Intel web

The browser app is React Native Web, served by Next from [apps/web-desktop](../../apps/web-desktop). It relays to the API on the same origin. Shared screens live in [packages/intel](../../packages/intel), tokens and primitives in [packages/ui](../../packages/ui), and the typed Query client in [packages/api-client](../../packages/api-client).

Start with the [web contract](code/intel-web.md), then the [UI rules](code/react-ui.md) and [browser checks](code/react-ui-testing.md). The [request figure](../diagrams/intel-web.html) shows the relay and readiness boundaries.

## Terminal clients and the retained desk

The supported network clients are Venture plus Atlas, Brain, Jobs, Tools, Models, and Profile. They are thin HTTP clients. The older Ratatui desk remains in the tree for compatibility reads and for the embedded test gate. Its layout and verification rules are the [TUI design spec](code/tui-design-spec.md), [components](code/tui-components.md), and [verification](code/tui-verification.md). The retained Profile dashboard has its own [analytics contract](code/profile-analytics-dashboard.md) and [search contract](code/profile-dashboard-and-search.md).

Research turns are retired. Historical list and show commands remain. The [rollout status](code/fleetworks.md#rollout-status) records what is current.

## Where the code lives

The [repository map](../index.md#directory-map) is the directory index. The parts you open most often:

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
| [.planning](../../.planning) | Named task plans |
| [.opencode](../../.opencode) and [.agents/skills](../../.agents/skills) | Agent roles and skills |

The [core map](code/core.md) is the module index inside the domain crate.

## Change, check, and release

Before editing, read the [contributor workflow](code/contributing.md) and [conventions](code/conventions.md). [Agent execution](code/agent-workflow.md) is the scope and evidence policy. OpenCode roles and commands are in the [OpenCode guide](code/opencode-v2.md). Multi-step work uses a [named plan](code/planning-artifacts.md#working-format).

Hosted checks are split by destination. A pull request into any branch other than `main` runs the [Fleetworks scopes](code/ci.md#fleetworksyml--scoped-branch-checks) selected for the changed paths. The four-platform embedded compatibility builds run when commits land on `main`, or by dispatch. The [CI guide](code/ci.md) lists every workflow. [Non-main merge checks](code/ci.md#checks-for-a-merge-that-is-not-into-main) are the short version.

One SemVer covers the workspace. It lives in `version.txt` and `Cargo.toml`. Git release tags use a leading `v`. Server images use the version without that prefix, plus a `:local` alias. [Versioning](code/versioning.md) is the commit and release policy.

## Docs, tags, and history

[Tags](tags.md) connect a concern across pages and source comments. [Wiki maintenance](maintenance.md) is how to add or refresh a page without creating a second explanation. Dated investigations stay in the [archive](../archive/index.md).
