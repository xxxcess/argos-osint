# Fleetworks

[Code route](index.md) · [Architecture](architecture.md) · [Deployment guide](../../../deploy/README.md)

Fleetworks Desktop is the API-only macOS control hub. Its six sidebar destinations are DB Seed & Train, News Cycles, Brain Search, OSINT Tools, Model Assignments and Jobs & Logs. The gateway header opens Workstation Status; Setup Gateway opens a URL-only connectivity flow. Backend resources remain shared within permissions, while activity is attributed to the registered workstation. Start with [native connections](native-connections.md), [protected identity](native-workspaces.md), the [design system](native-design-system.md) and [native build/fixture guide](../../../apps/fleetworks-desktop/README.md). App launch never launches the backend or Intel Web.

Linux domain services share the network datastore. Jobs exclusively owns its SQLite operational logs and embedded RocksDB queue; [actor discovery](actor-search.md) has a separate private Tantivy service. The [isolated native-review deployment](../../../deploy/README.md) builds backend-only images and leaves web/client launch independent. Backend network readers accept explicit current-format configuration and saved credentials; they do not migrate historical files or revive environment provider keys during ordinary reads.

Venture, the six Fleetworks terminal clients and the legacy `argos` executable are retired. Use the native app, Intel Web and direct Docker deployment. Historical terminal evidence is retained under the [retirement disposition](../../archive/cli-retirement.md); it is not a current startup recipe.

[![Fleetworks service boundaries](../../diagrams/fleetworks.svg)](../../diagrams/fleetworks.html)

## Build and connect the native app

```sh
python3 scripts/fleetworks_candidate.py build-native --target host --profile debug
```

The helper prints the complete `.app` and manifest paths. Add `--open` only for an intended GUI launch. Provision an exact-origin protected Native token-file binding using the app executable's `--provision-gateway-credential --api-origin <origin> --api-token-file <private-file>` mode. In Setup Gateway enter that URL, check authentication/compatibility and register the installation. The mode exits without GUI/network/service startup. [Connection flow](native-connections.md)

Protected settings initialize without creating a default child. The opaque backend workstation ID is the Connect ID; optional mutable IP metadata does not define identity. New contexts and activity queries contain no workspace narrowing. Existing folders and immutable historical origins remain preserved. [Identity ownership](native-workspaces.md)

## Launch the Compose stack

Use the [standalone backend recipe](../../../deploy/README.md#isolated-native-backend-review): provision with `deploy/native_review.py init --root-env`, build the `backend` Bake group, then start `compose.native-review.yml` directly with `--env-file compose.env` and its unique project. The editable env file is private and Git-ignored at the repository root. This creates separate deployment data and purpose-scoped credentials without mounting a native child. Reuse that exact Compose selection for status, logs, restart and stop. Stop preserves data; no startup resets configuration or rotates credentials.

Optional Intel Web and Atlas services require explicit deployment configuration. The browser authenticates its Connect form with an already registered Connect ID; the ID alone grants no authority. Use native News Cycles to submit collection work when a real worker is advertised. [Deployment](../../../deploy/README.md) · [Atlas](../../../deploy/atlas-dispatch.md)

## Run the network API and workers

Build the service host with network storage enabled.

```sh
cargo build --locked -p argos-api-host --no-default-features --features server,nlp,classifier
```

The service host compiles the embedded RocksDB queue. Linux source builds need `libclang-dev`; macOS builds need Xcode or its Command Line Tools. Set `LIBCLANG_PATH` when using a nonstandard LLVM installation. Client-only builds do not compile this engine.

Set `FLEETWORKS_WORKSPACE` and `ARGOS_HOME` to the same absolute workspace path. Set `FLEETWORKS_STORAGE_URL` to the SurrealDB `ws://` origin on a private network or `wss://` origin for TLS. The namespace, database, and runtime username default to `fleetworks`. Use `FLEETWORKS_STORAGE_NAMESPACE`, `FLEETWORKS_STORAGE_DATABASE`, and `FLEETWORKS_STORAGE_USERNAME` to override them.

Run `argos-api-host --role bootstrap` before serving against the authenticated datastore. It applies the schema and creates the database-scoped Editor account. Set `FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE` and `FLEETWORKS_STORAGE_PASSWORD_FILE` to files containing the bootstrap and runtime passwords. The root username defaults to `root` and can be set with `FLEETWORKS_STORAGE_ROOT_USERNAME`. Use a randomly generated 32–256 character runtime password with letters, digits, or `-._~`. Keep the runtime password file across restarts. A new in-memory server starts with a new database identity.

Then run the host with `--role api`, `--role worker` and `--role brief-worker` using the runtime credentials. The API additionally needs a purpose-scoped `FLEETWORKS_API_CREDENTIALS_FILE` manifest, matching access scope/epoch and `FLEETWORKS_API_ALLOWED_HOSTS`. The latter is a comma-separated list of exact Host authorities, such as `127.0.0.1:8080,localhost:8080`. Set `FLEETWORKS_API_ALLOWED_ORIGINS` to the allowed browser origins. The API binds to loopback unless `FLEETWORKS_API_BIND` is set explicitly. Both worker roles run without an HTTP listener.

Standalone private services use the same workspace and network storage settings. Start each with `argos-api-host --role <role>` and set `FLEETWORKS_<SERVICE>_BIND` and `FLEETWORKS_<SERVICE>_ALLOWED_HOSTS`, where `<SERVICE>` is the uppercase role name. Share each bearer file between the service and its callers. The direct Compose recipe provisions these connections explicitly.

| Role | Responsibility | Shared bearer variable |
| --- | --- | --- |
| `brain` | Actor dossiers, retained briefs and actions, Related articles, topology | `FLEETWORKS_INTERNAL_TOKEN_FILE` |
| `tools` | Tool catalog, submission and results | `FLEETWORKS_TOOLS_TOKEN_FILE` |
| `control` | Model defaults, Profile and configuration exports | `FLEETWORKS_CONTROL_TOKEN_FILE` |
| `jobs` | Job reads, retry/cancellation, and SQLite operational logs | `FLEETWORKS_JOBS_TOKEN_FILE` |
| `execution` | Retained API-key model attempts | `FLEETWORKS_EXECUTION_TOKEN_FILE` |

Set `FLEETWORKS_BRAIN_ORIGIN`, `FLEETWORKS_TOOLS_ORIGIN`, `FLEETWORKS_CONTROL_ORIGIN`, and `FLEETWORKS_JOBS_ORIGIN` on the gateway. Set `FLEETWORKS_EXECUTION_ORIGIN` on Brain, the worker and brief worker. The gateway owns live event streams; the brief worker owns stage validation and publication. Profile identifies Control's process, while gateway readiness identifies the gateway.

Execution resolves captured provider accounts from workspace configuration and streams progress and results. Its callers load route metadata without provider API keys and cannot write `auth.json`; local endpoints and the ChatGPT subscription execute in the caller. Role-default edits apply to new work, and endpoint changes invalidate captured routes. Callers own retries and validation.

With the Atlas overlay, `atlas-worker` consumes Jobs-owned embedded queue deliveries after SurrealDB records the transitional job and outbox transaction. Stored job revisions and database identity govern cancellation, retries and completed-run replay. The `nlp` and `classifier` roles accept authenticated requests. Use explicit native readiness maintenance to train a Natural candidate from a staged immutable dataset. Activation is a separate confirmed operation; neither operation supplies Rust-BERT inference.

## Rollout status

| Area | Implemented behavior |
| --- | --- |
| Native app and workstation identity | URL-only setup, provisioned exact-origin credentials, stable workstation Connect ID and scoped request context |
| Direct Docker backend | Independently provisioned purpose credentials, private services, workers and persistent stores |
| Atlas | Durable dispatch and replay, separate NLP/Rust-BERT services, shared daily NewsAPI quota in SurrealDB |
| Brain and Intel | Canonical actor discovery, dossiers, retained briefs and publication recovery; explicit seed maintenance |
| Tools and Model Assignments | Provider/service cards, protected inline keys, shared routing defaults and confirmed revisions |
| Jobs & Logs | Workstation membership before rows/counts/pagination, private-safe logs and explicitly authorized actions |

Installed-app, matching-hardware accessibility, both Mac architectures and real backend integration require their own recorded evidence. Retirement was explicitly authorized; it does not close those verification gaps or the separate production scheduler migration. [Native verification](native-verification.md) · [CI](ci.md#fleetworksyml--scoped-branch-checks)

## Verification

Use [the network smoke](../../../scripts/fleetworks_network_smoke.py) with the built host and the repository-pinned SurrealDB 3.3.4 binary. It creates isolated memory and disk fixtures and checks authenticated startup, retained routes and restart behavior.

## Operational persistence migration

The Jobs process exclusively opens `FLEETWORKS_JOBS_SQLITE_PATH` (default: `<workspace>/storage/jobs/jobs.sqlite3`). Compose mounts `storage/jobs/` only into Jobs at `/var/lib/fleetworks-jobs`; other processes use its authenticated HTTP endpoint. WAL mode and FULL synchronization persist acknowledged writes. Stop retains the database.

Standalone operational logs use SQLite; investigation traces and transactional job lifecycle events retain their domain storage. Job scheduling, leases, attempts, recovery and Profile telemetry still use SurrealDB. The SQLite crate supplies scheduling primitives, but production scheduler cutover is pending the domain handoff/fencing port. Setup uses a fresh deployment with new operational stores and no existing-job migration prerequisite.

Jobs already owns the embedded RocksDB delivery queue. The target pairs it with SQLite job state. See [ownership, rollout scopes and acceptance](jobs-persistence.md) before treating the scheduler migration as complete.
