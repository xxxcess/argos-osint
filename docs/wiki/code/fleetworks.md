# Fleetworks

[Code route](index.md) · [Implementation plan](../../../.planning/2026-11-09-fleetworks-overhaul/task_plan.md)

Venture initializes an isolated workspace, imports selected existing configuration on request, and owns the local Compose stack. The terminal clients and Intel web relay communicate with the authenticated Fleetworks API. Service processes open the network datastore according to their roles.

[![Fleetworks service boundaries](../../diagrams/fleetworks.svg)](../../diagrams/fleetworks.html)

## Try the workspace and clients

From a checkout, build the launcher and six sibling executables.

```sh
cargo build --locked -p fleetworks-venture -p fleetworks-cli --bins
target/debug/venture --workspace ./review-workspace init
target/debug/venture --workspace ./review-workspace client profile workspace
```

Workspace selection uses `--workspace`, `FLEETWORKS_WORKSPACE`, then `.fleetworks` under the current directory. Venture displays the absolute workspace path. The initial directories are `datasets/`, `exports/`, `config/`, `secrets/`, `models/`, `logs/`, and `storage/`.

Use `config/workspace.toml` for `[datasets]` bindings and `[environment]` defaults. Dataset paths are relative to the workspace. Use `secrets/environment.toml` for private environment defaults. An explicitly set process variable takes precedence over either file.

For an explicit configuration import, run `venture --workspace ./review-workspace import --from "$HOME/.argos"`. Venture copies selected provider API-key configuration and model defaults to the destination workspace and reports only counts. A conflicting destination is an error. The source configuration and application data stay in the original home.

## Connect to the API

The clients default to `http://127.0.0.1:8080`. Pass `--api-origin` or set `FLEETWORKS_API_ORIGIN` to select the API listener. The API bearer can come from `--api-token-file`, `FLEETWORKS_API_TOKEN`, `FLEETWORKS_API_TOKEN_FILE`, or the workspace's `secrets/api-token`. Use the same token as the API when it requires one. Unauthenticated HTTP is restricted to loopback. Remote connections require HTTPS and a bearer token.

See the [CLI command reference](../../../apps/fleetworks-cli/README.md) for route-specific commands and their API requirements. Use `venture status` for storage-backed readiness and `venture client <app> --help` for an individual CLI.

## Launch the Compose stack

From the repository root, run `target/debug/venture --workspace ./review-workspace up` with a local Docker engine and Docker Compose installed. Venture builds and starts the API, private Brain, Tools, Control and Jobs services, brief worker, private model execution, other background workers, production web relay and disk-backed SurrealDB. The launcher creates private local credentials and records the Compose project it owns.

Open the web address Venture prints. The default is `http://127.0.0.1:3000`. Set `FLEETWORKS_WEB_PORT` or `FLEETWORKS_API_PORT` to remap the loopback ports. Use `venture status`, `venture logs --follow`, and `venture stop` with the same workspace. Stop retains the database directory.

Use `venture web` to start the web stack and open the interactive Jobs and Logs client. `venture web --terminal logs` follows Compose output instead. The same storage, Compose file and Atlas options apply as for `up`. To attach Jobs and Logs to an existing stack, use `venture client jobs tui`. The line-oriented session refreshes jobs, opens details and logs, and confirms retry or cancel commands. Exiting either terminal presentation leaves the stack running.

Use `up --dev` or `web --dev` to run Next development mode from this checkout. Venture remembers the selected profile for status, logs and stop. Use `up --no-build` for preloaded candidate images. [Development profile](../../../deploy/README.md)

Add `--atlas` to start the private RabbitMQ, Compromise NLP, and Rust-BERT classifier services. Submit a durable cycle with `venture client atlas start --request-id <id>`. Use a new request ID for each new collection. The [Atlas deployment guide](../../../deploy/atlas-dispatch.md) explains the provider and model files required for a live cycle.

Use `up --storage host-disk` or `up --storage host-memory` to let Venture own an installed SurrealDB 3.3.4 host process. Use `up --storage external` with an explicit `FLEETWORKS_STORAGE_URL` and root credential file for an existing disk-backed server. See the [deployment guide](../../../deploy/README.md) for environment options and manual Compose startup.

## Run the network API and workers

Build the service host with network storage enabled.

```sh
cargo build --locked -p argos-api-host --no-default-features --features server,nlp,classifier
```

Set `FLEETWORKS_WORKSPACE` and `ARGOS_HOME` to the same absolute workspace path. Set `FLEETWORKS_STORAGE_URL` to the SurrealDB `ws://` origin on a private network or `wss://` origin for TLS. The namespace, database, and runtime username default to `fleetworks`. Use `FLEETWORKS_STORAGE_NAMESPACE`, `FLEETWORKS_STORAGE_DATABASE`, and `FLEETWORKS_STORAGE_USERNAME` to override them.

Run `argos-api-host --role bootstrap` before serving against the authenticated datastore. It applies the schema and creates the database-scoped Editor account. Set `FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE` and `FLEETWORKS_STORAGE_PASSWORD_FILE` to files containing the bootstrap and runtime passwords. The root username defaults to `root` and can be set with `FLEETWORKS_STORAGE_ROOT_USERNAME`. Use a randomly generated 32–256 character runtime password with letters, digits, or `-._~`. Keep the runtime password file across restarts. A new in-memory server starts with a new database identity.

Then run the host with `--role api`, `--role worker` and `--role brief-worker` using the runtime credentials. The API additionally needs `FLEETWORKS_API_TOKEN_FILE` and `FLEETWORKS_API_ALLOWED_HOSTS`. The latter is a comma-separated list of exact Host authorities, such as `127.0.0.1:8080,localhost:8080`. Set `FLEETWORKS_API_ALLOWED_ORIGINS` to the allowed browser origins. The API binds to loopback unless `FLEETWORKS_API_BIND` is set explicitly. Both worker roles run without an HTTP listener.

For a standalone private Brain process, use `argos-api-host --role brain` with the same network storage settings and set `FLEETWORKS_BRAIN_BIND` and `FLEETWORKS_BRAIN_ALLOWED_HOSTS` as needed. Provide a shared `FLEETWORKS_INTERNAL_TOKEN_FILE` to Brain and the gateway API, and set `FLEETWORKS_BRAIN_ORIGIN` on the gateway to the private service origin. The Compose base stack provisions a separate `brain-token` for this connection. Brain handles actor dossiers, retained brief details and actions, Related articles, and topology operations. The brief worker owns stage validation and publication. The gateway owns live event streams.

The private Tools role serves the catalog, submission and result routes. In a standalone deployment, set `FLEETWORKS_TOOLS_ORIGIN` on the gateway and point its `FLEETWORKS_TOOLS_TOKEN_FILE` and the Tools process at the same private bearer file. Venture provisions this automatically. Private Control owns the Model defaults and Profile routes. Private Jobs owns job and log reads, retry and cancellation. Each has a separate origin and bearer. Profile reports the Control process instance, while gateway readiness reports the gateway instance. The brief worker sends eligible claims, inferences and links chat attempts to the private execution role over an authenticated stream. The role resolves the captured account from workspace configuration and returns typed progress and results. An edit to role defaults applies to new work. A provider endpoint change invalidates its captured route. Retries and validation remain with the brief worker.

With the Atlas overlay, the private `atlas-worker` consumes durable RabbitMQ deliveries. A transaction in SurrealDB records the job and outbox state before dispatch. Cancellation, retries, and replay of a completed run follow the stored job revision and database identity. The broker carries delivery references and never owns the application records. The private `nlp` and `classifier` roles accept bounded authenticated requests. Use `venture train-legacy` for a separate Natural classifier training run against a snapshot of the selected CSV.

## Rollout status

| Area | Current branch |
| --- | --- |
| Venture and workspace | Workspace initialization, explicit key/default import, owned lifecycle and client launch |
| Atlas | Durable submission, worker dispatch, cancellation, retry and completed-run replay. The overlay serves NLP and Rust-BERT separately. Storage-backed NewsAPI tools share Atlas's daily NewsAPI quota in SurrealDB. |
| Brain and Intel | Private Brain owns retained actor, article brief, Related and topology HTTP operations. The dedicated brief worker runs the stages and replays their stored publication work. The gateway owns live streams. Brain supports actor search by exact name or indexed alias. Seed a fresh workspace with `venture seed-actors --file <workspace CSV>`. [Actor catalog guide](actor-catalog.md) |
| Tools, Models and Profile | The private Tools service runs authenticated manual calls and records durable results. Models reads and saves role defaults. Profile reports workspace and health and writes portable configuration exports on the server. |
| Interactive terminal | All six clients open a line-oriented session from `venture client <app>`. Use an explicit subcommand for one-shot output. Jobs and Logs supports details, retry and cancellation. |
| Service cutover | Network workers share per-provider model pacing and per-account admission. Focus Brief chat attempts use the private execution service. Other model transports, published image verification and compatibility Recon retirement remain open. |

The [integrated checkpoint at `4038049`](https://github.com/xxxcess/argos-osint/actions/runs/42781577487) passed native Linux and Intel macOS clients, real Intel Rust-BERT inference, ARM64 server verification, the private Brain relay, the Atlas quota contract, native RabbitMQ dispatch and packaged Compose on Linux ARM64 and AMD64. The [implementation plan](../../../.planning/2026-11-09-fleetworks-overhaul/task_plan.md) tracks the remaining acceptance work.

## Verification

Use [the network smoke](../../../scripts/fleetworks_network_smoke.py) with the built host and the repository-pinned SurrealDB 3.3.4 binary. It creates isolated memory and disk fixtures and checks authenticated startup, retained routes and restart behavior.
