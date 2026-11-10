# Fleetworks

[Code route](index.md) · [Implementation plan](../../../.planning/2026-11-09-fleetworks-overhaul/task_plan.md)

Venture initializes an isolated workspace, imports selected legacy configuration on request, and owns the local Compose stack. The terminal clients and Intel web relay communicate with the authenticated Argos API. Service processes open the network datastore according to their roles.

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

For an explicit legacy import, run `venture --workspace ./review-workspace import --from "$HOME/.argos"`. Venture copies selected provider API-key configuration and model defaults to the destination workspace and reports only counts. A conflicting destination is an error. The source configuration and application data stay in the original Argos home.

## Connect to the API

The clients default to `http://127.0.0.1:8080`. Pass `--api-origin` or set `FLEETWORKS_API_ORIGIN` to select the API listener. The API bearer can come from `--api-token-file`, `FLEETWORKS_API_TOKEN`, `FLEETWORKS_API_TOKEN_FILE`, or the workspace's `secrets/api-token`. Use the same token as the API when it requires one. Unauthenticated HTTP is restricted to loopback. Remote connections require HTTPS and a bearer token.

See the [CLI command reference](../../../apps/fleetworks-cli/README.md) for route-specific commands and their API requirements. Use `venture status` for storage-backed readiness and `venture client <app> --help` for an individual CLI.

## Launch the Compose stack

From the repository root, run `target/debug/venture --workspace ./review-workspace up` with a local Docker engine and Docker Compose installed. Venture builds and starts the API, private Brain service, background worker, production web relay and disk-backed SurrealDB. The launcher creates private local credentials and records the Compose project it owns.

Open the web address Venture prints. The default is `http://127.0.0.1:3000`. Set `FLEETWORKS_WEB_PORT` or `FLEETWORKS_API_PORT` to remap the loopback ports. Use `venture status`, `venture logs --follow`, and `venture stop` with the same workspace. Stop retains the database directory.

Use `venture web` to start the web stack and open the interactive Jobs and Logs client. `venture web --terminal logs` follows Compose output instead. The same storage, Compose file and Atlas options apply as for `up`. To attach Jobs and Logs to an existing stack, use `venture client jobs tui`. The line-oriented session refreshes jobs, opens details and logs, and confirms retry or cancel commands. Exiting either terminal presentation leaves the stack running.

Add `--atlas` to start the private RabbitMQ, Compromise NLP, and Rust-BERT classifier services. Submit a durable cycle with `venture client atlas start --request-id <id>`. Use a new request ID for each new collection. The [Atlas deployment guide](../../../deploy/atlas-dispatch.md) explains the provider and model files required for a live cycle.

Use `up --storage host-disk` or `up --storage host-memory` to let Venture own an installed SurrealDB 3.3.4 host process. Use `up --storage external` with an explicit `FLEETWORKS_STORAGE_URL` and root credential file for an existing disk-backed server. See the [deployment guide](../../../deploy/README.md) for environment options and manual Compose startup.

## Run the network API and workers

Build the service host with network storage enabled.

```sh
cargo build --locked -p argos-api-host --no-default-features --features server,nlp,classifier
```

Set `FLEETWORKS_WORKSPACE` and `ARGOS_HOME` to the same absolute workspace path. Set `FLEETWORKS_STORAGE_URL` to the SurrealDB `ws://` origin on a private network or `wss://` origin for TLS. The namespace, database, and runtime username default to `fleetworks`. Use `FLEETWORKS_STORAGE_NAMESPACE`, `FLEETWORKS_STORAGE_DATABASE`, and `FLEETWORKS_STORAGE_USERNAME` to override them.

Run `argos-api-host --role bootstrap` before serving against the authenticated datastore. It applies the schema and creates the database-scoped Editor account. Set `FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE` and `FLEETWORKS_STORAGE_PASSWORD_FILE` to files containing the bootstrap and runtime passwords. The root username defaults to `root` and can be set with `FLEETWORKS_STORAGE_ROOT_USERNAME`. Use a randomly generated 32–256 character runtime password with letters, digits, or `-._~`. Keep the runtime password file across restarts. A new in-memory server starts with a new database identity.

Then run `argos-api-host --role api` and `argos-api-host --role worker` with the runtime credentials. The API additionally needs `FLEETWORKS_API_TOKEN_FILE` and `FLEETWORKS_API_ALLOWED_HOSTS`. The latter is a comma-separated list of exact Host authorities, such as `127.0.0.1:8080,localhost:8080`. Set `FLEETWORKS_API_ALLOWED_ORIGINS` to the allowed browser origins. The API binds to loopback unless `FLEETWORKS_API_BIND` is set explicitly. The worker runs the background pools without an HTTP listener.

For a standalone private Brain process, use `argos-api-host --role brain` with the same network storage settings and set `FLEETWORKS_BRAIN_BIND` and `FLEETWORKS_BRAIN_ALLOWED_HOSTS` as needed. Provide a shared `FLEETWORKS_INTERNAL_TOKEN_FILE` to Brain and the gateway API, and set `FLEETWORKS_BRAIN_ORIGIN` on the gateway to the private service origin. The Compose base stack provisions a separate `brain-token` for this connection. Brain handles actor dossiers, retained brief details and actions, Related articles, and topology operations. The existing worker owns background brief execution and the gateway retains live event streams.

The default `combined` role supports the legacy local API and its worker pools. With the Atlas overlay, the private `atlas-worker` consumes durable RabbitMQ deliveries. A transaction in SurrealDB records the job and outbox state before dispatch. Cancellation, retries, and replay of a completed run follow the stored job revision and database identity. The broker carries delivery references and never owns the application records. The private `nlp` and `classifier` roles accept bounded authenticated requests. Use `venture train-legacy` for a separate Natural classifier training run against a snapshot of the selected CSV.

## Rollout status

| Area | Current branch |
| --- | --- |
| Venture and workspace | Workspace initialization, explicit key/default import, owned lifecycle and client launch |
| Atlas | Durable submission, worker dispatch, cancellation, retry and completed-run replay. The overlay serves NLP and Rust-BERT separately. Workers share a daily news fetch counter in SurrealDB. |
| Brain and Intel | Private Brain owns retained actor, article brief, Related and topology HTTP operations. Background brief execution and live streams still use the existing worker and gateway. |
| Tools, Models and Profile | Catalog, role-default and workspace/health reads through the client suite. Tool execution and full configuration/export parity remain open. |
| Interactive terminal | Line-oriented Jobs and Logs session with details, retry and cancellation. Other clients expose commands. Full legacy TUI parity remains open. |
| Service cutover | Further domain extraction, cross-worker model admission and cooldowns, packaged release verification and Recon retirement remain open. |

The [hosted checkpoint at `2934b1f`](https://github.com/xxxcess/argos-osint/actions/runs/42763558953) passed native Linux and Intel macOS clients, real Intel Rust-BERT inference, ARM64 server verification, native RabbitMQ dispatch and Compose configuration checks. The packaged Compose smoke failed during managed database startup at that revision. The BusyBox entrypoint fix landed in `fea751a`, and [ARM64 packaged Compose passed at that revision](https://github.com/xxxcess/argos-osint/actions/runs/42777729999). Consult the [branch runs](https://github.com/xxxcess/argos-osint/actions?query=branch%3Afeat%2Fatlas-zero-shot) for the result on a newer revision. The [implementation plan](../../../.planning/2026-11-09-fleetworks-overhaul/task_plan.md) tracks the remaining acceptance work.

## Verification

Use [the network smoke](../../../scripts/fleetworks_network_smoke.py) with the built host and the repository-pinned SurrealDB 3.3.4 binary. It creates isolated memory and disk fixtures and checks authenticated startup, retained routes and restart behavior.
