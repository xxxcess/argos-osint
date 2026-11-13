# Fleetworks

<p align="center">
  <img src="assets/brand/fleetworks-logo.jpg" alt="Fleetworks emblem with five gold rays, an ivory disc, and a gold crescent" width="960">
</p>

Fleetworks collects public-source intelligence, connects evidence to actors and articles, and prepares cited Focus Briefs. Fleetworks Desktop is a native Rune macOS control hub connected to an independently operated Linux backend. Intel Web provides the browser desktop. Backend services are deployed directly with Docker Compose.

[Native app](apps/fleetworks-desktop/README.md) · [Connections](docs/wiki/code/native-connections.md) · [Setup and usage](docs/wiki/code/fleetworks.md) · [Deployment](deploy/README.md) · [Wiki](docs/index.md) · [Releases](docs/wiki/code/versioning.md)

## Quick start

### Native macOS app

Install the pinned Rust **1.95.0** toolchain and Xcode or its Command Line Tools. Run from the repository root to build and package a debug `.app` for the physical Mac:

```sh
python3 scripts/fleetworks_candidate.py build-native --target host --profile debug
```

[![Provision and connect](docs/diagrams/native-connections.svg)](docs/diagrams/native-connections.html)

The helper prints the app and manifest paths. Add `--open` to launch it. The app opens offline setup and connects to an existing gateway; backend and Intel Web startup are separate operator actions.

**Native setup:** Gateway URL → operator approval once → macOS Keychain → authenticated reconnect. No user-managed token file is needed. [Pairing and recovery](docs/wiki/code/native-connections.md#pairing-and-keychain-agreed-target) · [Verification status](.planning/2026-11-12-native-pairing-implementation/acceptance.md)

1. Start the backend with the [Docker Compose quick start below](#backend-for-the-native-app). Backend deployment requires Docker and Docker Compose **v2.20 or newer**.
2. Open **Setup Gateway**, enter the gateway URL and choose **Connect**. Give its pairing request ID and approval code to the gateway operator for [approval](docs/wiki/code/native-connections.md#operator-approval-and-revocation).
3. The app authenticates the approved access, saves it in macOS Keychain and registers the installation. Workstation Status shows its stable opaque Connect ID; later connections reuse saved access.

Normal startup initializes protected settings in `~/.fleetworks` without creating a default child. IP is optional metadata; folders do not select identity or partition shared actors, model defaults and provider accounts. Existing folders and historical receipts remain preserved. See [workstation identity](docs/wiki/code/native-workspaces.md).

Enter the selectable Connect ID from **Workstation Status** into Intel Web's existing Connect form. The browser uses its separately provisioned WebRelay credential and session; the Connect ID alone grants no authority. See [connection and browser handoff](docs/wiki/code/native-connections.md#browser-handoff).

### Backend for the native app

Start Docker Desktop or your local Docker engine, then run these commands from the repository root. The backend includes the gateway, private services, workers, SurrealDB, Jobs storage and actor search.

**First-time setup:** create the `native-local` deployment and build its three backend images. Run initialization once; later starts reuse its credentials and data.

```sh
export PLATFORM="$(python3 scripts/fleetworks_verify.py platform --format docker)"
python3 deploy/native_review.py init --name native-local --api-port 18091 --platform "$PLATFORM" --root-env
export IMAGE_TAG="$(python3 deploy/native_review.py value --name native-local --field image_tag)"
export VERSION="$(tr -d '[:space:]' < version.txt)"
docker buildx bake -f deploy/docker-bake.hcl --load backend
```

Edit `compose.env` at the repository root to update local Compose settings. It is private and ignored by Git. The deployment's generated env path points to this same file, so both locations use the same settings. Keep its generated access scope, epochs and storage path with the existing credentials and data.

**Start the backend:** use this command after setup and whenever you want to start it again. It also works from a fresh terminal at the repository root.

```sh
docker compose --env-file compose.env \
  -f deploy/compose.native-review.yml --project-name fleetworks-native-review-native-local \
  up --no-build --pull never --detach --wait
```

Enter `http://127.0.0.1:18091` in **Setup Gateway** and have the operator approve the displayed pairing request. Existing stacks need an independently provisioned Operator credential; see [operator setup](deploy/README.md#credentials). This backend runs independently of the app. Atlas collection uses the [additional overlay below](#collect-articles-with-atlas); Intel Web is an optional separate deployment.

Rerun the start command to apply edited Compose settings. To inspect or stop the backend, use the same command prefix with `ps`, `logs --follow` or `down` in place of `up --no-build --pull never --detach --wait`. Stop preserves backend data. For concurrent deployments, omit `--root-env`, choose a different name and API port, and use that deployment's matching env file and project. [Full deployment guide](deploy/README.md#isolated-native-backend-review)

Venture, the six terminal clients and the legacy `argos` executable are retired. Their historical evidence remains available in the [retirement disposition](docs/archive/cli-retirement.md). Backend service executables and administrative support tools remain supported.

### Collect articles with Atlas

Provision provider credentials, the Atlas worker, private NLP and a Rust-BERT model for the container architecture. Enable the separately configured [Atlas Compose overlay](deploy/atlas-dispatch.md). Submit through native News Cycles after connecting and registering; submission requires an advertised durable worker and permission. A job receipt confirms acceptance, while Jobs reports progress and completion. The [Atlas deployment guide](deploy/atlas-dispatch.md) lists the files and settings.

## Applications and services

| Area | What you can do |
| --- | --- |
| DB Seed & Train | Trigger configured actor seed, search-index build and Natural candidate training; acceptance is distinct from completion. |
| News Cycles and Brain Search | Queue cycles, inspect workstation cycle history, discover actors and read dossiers. |
| OSINT Tools and Model Assignments | Read service/provider cards, provision keys and edit supported model routing with confirmed revisions. |
| Workstation Status and Setup Gateway | Inspect identity/readiness/API-host resources; pair this Mac and reconnect with saved Keychain access. |
| Jobs & Logs | Read the bounded workstation queue and selected-job safe operational summaries. |
| Intel Web | Browse the Bulletin, Focus Briefs, Related articles, actor dossiers and topology behind the Connect/session gate. |

[![Fleetworks service boundaries](docs/diagrams/fleetworks.svg)](docs/diagrams/fleetworks.html)

The native app uses shared Rust contracts and an origin-pinned API client. Axum serves the gateway; Next.js runs the React Native Web desktop through a same-origin relay with a separate credential. Linux services own inference, indexing and storage: SurrealDB holds domain records and current scheduling state, Jobs owns SQLite operational logs and its embedded RocksDB delivery queue, and private Tantivy serves actor discovery. The Atlas overlay runs Compromise NLP and Rust-BERT behind private endpoints. [Architecture](docs/wiki/code/architecture.md) · [Persistence rollout](docs/wiki/code/jobs-persistence.md)

Source implementation, fixture checks and packaged builds are separate from installed-app, physical-Mac accessibility, real backend integration and both-architecture verification. Production scheduler migration retains its own acceptance gates. Executable retirement does not establish installed-app or dual-Mac parity. See [rollout status](docs/wiki/code/fleetworks.md#rollout-status), [native verification](docs/wiki/code/native-verification.md) and [CI](docs/wiki/code/ci.md) for the applicable checks.

## Development

Use the [contributor workflow](docs/wiki/code/contributing.md) to select checks for the code you changed. Preview the hosted checks for the current commit with the scope planner.

```sh
python3 scripts/fleetworks_verify.py plan --base "$(git rev-parse HEAD^)" --head "$(git rev-parse HEAD)"
```

### Iteration loops

Choose the shortest loop for the changed surface. The native app/fixture graph is engine-free; backend storage tests can link embedded SurrealDB/RocksDB as QA harnesses. Service edits require the affected Linux image. [Visual artifact/path map](docs/wiki/code/build-artifacts.md)

[![Fleetworks development loops](docs/diagrams/dev-loops.svg)](docs/diagrams/dev-loops.html)

| Change | Loop |
| --- | --- |
| Rune app, client contracts or workstation manager | Scoped host debug tests, selected same-view Runtime/PNG/tree fixtures, then `python3 scripts/fleetworks_candidate.py build-native --target host --profile debug`. |
| Service roles (`argos-api-host`) | Rebuild the `runtime` image; the native-review `backend` Bake group builds backend-only images. BuildKit caches dependencies. |
| Web app and `packages/*/src` | Run the separately configured Next development server; use isolated repository Playwright output for browser checks. |
| Dependencies, Dockerfile, Compose or private search | Build the affected image with `docker buildx bake -f deploy/docker-bake.hcl <target>`, then use its isolated deployment checks. |
| Packaging or release | Push and read the hosted CI results for that revision. |

Run host checks first, then the affected container loop, then hosted CI. [Full loop guide](docs/wiki/code/contributing.md#development-loops-local-cli-and-docker)

Agents use `python3 scripts/agent_cargo.py … --locked` and shared `target/agents/debug`; add `--agent` to native build/fixture helpers. Selected test targets can enable extra dependency features, so a smaller `-p` selection may rebuild large dependencies. Choose the loop and resource preflight from [contributing](docs/wiki/code/contributing.md#verification-ownership-and-host-resources); preserve reusable outputs using [cache hygiene](docs/wiki/code/contributing.md#build-cache-hygiene).

Web contributors can find setup and verification in the [Intel web guide](docs/wiki/code/intel-web.md) and the [React test guide](docs/wiki/code/react-ui-testing.md). The [CI guide](docs/wiki/code/ci.md) covers hosted platform and package verification. Agents start at [AGENTS.md](AGENTS.md).

For more, browse the [concepts](docs/wiki/concepts/index.md), [code map](docs/wiki/code/index.md), or [diagram catalog](docs/diagrams.md).
