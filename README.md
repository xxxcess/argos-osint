# Fleetworks

<p align="center">
  <img src="assets/brand/fleetworks-logo.jpg" alt="Fleetworks emblem with five gold rays, an ivory disc, and a gold crescent" width="960">
</p>

Fleetworks collects public-source intelligence, connects evidence to actors and articles, and prepares cited Focus Briefs. Fleetworks Desktop is a native Rune macOS control hub connected to an independently operated Linux backend. Intel Web provides the browser desktop; Venture retains deployment and terminal-client compatibility operations.

[Native app](apps/fleetworks-desktop/README.md) · [Connections](docs/wiki/code/native-connections.md) · [Setup and usage](docs/wiki/code/fleetworks.md) · [Deployment](deploy/README.md) · [Wiki](docs/index.md) · [Releases](docs/wiki/code/versioning.md)

## Quick start

### Native macOS app

Install the pinned Rust **1.95.0** toolchain and Xcode or its Command Line Tools. Run from the repository root to build and package a debug `.app` for the physical Mac:

```sh
python3 scripts/fleetworks_candidate.py build-native --target host --profile debug
```

The helper prints the app and manifest paths. Add `--open` to launch it. The app opens offline setup and connects to an existing gateway; backend and Intel Web startup are separate operator actions.

1. Provision a backend using the [isolated native-review recipe](deploy/README.md#isolated-native-backend-review), or use the operator stack below. Backend deployment requires Docker and Docker Compose **v2.20 or newer**.
2. In **Workstation**, add a named connection with the gateway origin and its private Native-purpose token file. **Test** checks compatibility; **Connect** establishes the connection. Saved profiles keep protected file references, and reconnect is explicit.
3. Select a current LAN/VPN interface and an owned workspace to register request context. Activity and mutations use that registration and the credential's permissions.

Native workspaces are protected children of `~/.fleetworks`, with a `default` child created on first use. Selecting a child changes request origin and activity scope; actors, model defaults and provider accounts remain shared backend resources. See [workspace ownership](docs/wiki/code/native-workspaces.md).

When the gateway advertises an Intel Web URL, **Open Intel Web** opens its Connect form and copies the registered Connect ID. The browser uses its own server credential and session; the Connect ID alone is not a credential. See [connection and browser handoff](docs/wiki/code/native-connections.md#browser-handoff).

### Operator and Intel Web stack

Venture remains available for deployment and retained CLI workflows. Use a separate operator workspace from the native container:

```sh
cargo build --locked -p fleetworks-venture -p fleetworks-cli --bins
./target/debug/venture --workspace ./.fleetworks-backend up --storage compose-disk
./target/debug/venture --workspace ./.fleetworks-backend status
```

Venture initializes credentials and starts the stack. The gateway defaults to `http://127.0.0.1:8080`; Intel Web defaults to `http://127.0.0.1:3000` and requires a registered Connect ID. Select `.fleetworks-backend/secrets/native-api-token` in the native connection editor. The separate operator credential supports diagnostics; it does not grant native registration or mutation permissions.

Use the same operator workspace for status, logs and stop. Concurrent stacks need distinct workspaces, project names and ports. [Storage and operator setup](docs/wiki/code/fleetworks.md) · [Credential roles](deploy/README.md#credentials) · [CLI reference](apps/fleetworks-cli/README.md)

### Collect articles with Atlas

Provision provider credentials, the Atlas worker, private NLP and a Rust-BERT model for the container architecture. The operator stack enables its Atlas overlay with `up --atlas`. Submit through the native Atlas page after connecting and registering; submission requires an advertised durable worker and permission. A job receipt confirms acceptance, while Jobs reports progress and completion. The [Atlas deployment guide](deploy/atlas-dispatch.md) lists the files and settings.

## Applications and services

| Area | What you can do |
| --- | --- |
| Native Atlas and Brain | Submit collection cycles, browse articles, discover actors and read dossiers through the gateway. |
| Native Tools, Models and Profile | Submit permitted tools, inspect results, manage shared model defaults and provider configuration with confirmed revisions. |
| Workstation | Manage saved gateway connections, select a LAN/VPN address and inspect backend readiness. |
| Workspaces | Create, select and attach owned local children; track registration and guarded retirement/removal. |
| Jobs & Logs | Filter and page registered activity, inspect safe operational summaries and export bounded logs. Retry/cancel controls require explicit permission and a supported job state. |
| Intel Web | Browse the Bulletin, Focus Briefs, Related articles, actor dossiers and topology behind the Connect/session gate. |

[![Fleetworks service boundaries](docs/diagrams/fleetworks.svg)](docs/diagrams/fleetworks.html)

The native app uses shared Rust contracts and an origin-pinned API client. Axum serves the gateway; Next.js runs the React Native Web desktop through a same-origin relay with a separate credential. Linux services own inference, indexing and storage: SurrealDB holds domain records and current scheduling state, Jobs owns SQLite operational logs and its embedded RocksDB delivery queue, and private Tantivy serves actor discovery. The Atlas overlay runs Compromise NLP and Rust-BERT behind private endpoints. [Architecture](docs/wiki/code/architecture.md) · [Persistence rollout](docs/wiki/code/jobs-persistence.md)

Source implementation, fixture checks and packaged builds are separate from installed-app, physical-Mac accessibility, real backend integration and both-architecture verification. Terminal retirement and production scheduler migration retain their own acceptance gates. See [rollout status](docs/wiki/code/fleetworks.md#rollout-status), [native verification](docs/wiki/code/native-verification.md) and [CI](docs/wiki/code/ci.md) for the applicable checks.

## Development

Use the [contributor workflow](docs/wiki/code/contributing.md) to select checks for the code you changed. Preview the hosted checks for the current commit with the scope planner.

```sh
python3 scripts/fleetworks_verify.py plan --base "$(git rev-parse HEAD^)" --head "$(git rev-parse HEAD)"
```

### Iteration loops

Choose the shortest loop for the changed surface. Native app builds connect to the separately operated backend; service edits require the affected Linux image.

[![Fleetworks development loops](docs/diagrams/dev-loops.svg)](docs/diagrams/dev-loops.html)

| Change | Loop |
| --- | --- |
| Rune app, client contracts or workspace manager | Scoped host debug tests, selected same-view Runtime/PNG/tree fixtures, then `python3 scripts/fleetworks_candidate.py build-native --target host --profile debug`. |
| Venture or a `fleetworks-*` CLI | `cargo build --locked -p fleetworks-venture -p fleetworks-cli --bins`, unit tests, then `venture client <app>` against a running stack. No image rebuild. |
| Service roles (`argos-api-host`) | Rebuild the `runtime` image; the native-review `backend` Bake group builds backend-only images. BuildKit caches dependencies. |
| Web app and `packages/*/src` | `venture up --dev --storage local-net`. Source is mounted and reloads on save. |
| Dependencies, Dockerfile, Compose or private search | Build the affected image with `docker buildx bake -f deploy/docker-bake.hcl <target>`, then use its isolated deployment checks. |
| Packaging or release | Push and read the hosted CI results for that revision. |

Run host checks first, then the affected container loop, then hosted CI. [Full loop guide](docs/wiki/code/contributing.md#development-loops-local-cli-and-docker)

Agents use `python3 scripts/agent_cargo.py … --locked` and the shared `target/agents` debug cache; add `--agent` to native build/fixture helpers. Reuse the matching host/feature graph and preserve the user's `target/debug`. [Cache hygiene](docs/wiki/code/contributing.md#build-cache-hygiene)

Web contributors can find setup and verification in the [Intel web guide](docs/wiki/code/intel-web.md) and the [React test guide](docs/wiki/code/react-ui-testing.md). The [CI guide](docs/wiki/code/ci.md) covers hosted platform and package verification. Agents start at [AGENTS.md](AGENTS.md).

For more, browse the [concepts](docs/wiki/concepts/index.md), [code map](docs/wiki/code/index.md), or [diagram catalog](docs/diagrams.md).
