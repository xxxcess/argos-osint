# Fleetworks service deployment

This stack runs the production web desktop, authenticated API, private Brain and Intel routes, background worker pools, and one-shot database bootstrap. Docker Compose v2.20 or newer is required. The web desktop and API publish `127.0.0.1:3000` and `127.0.0.1:8080` by default. Database connections stay inside the Compose network for the managed profile. Use the [Atlas overlay](atlas-dispatch.md) for RabbitMQ dispatch and the private NLP service.

## Prepare a workspace

Run from the repository root. Use an absolute workspace path that Docker can mount. The numeric UID and GID must be able to read its owner-only credential files and write its data directories.

```sh
export FLEETWORKS_HOST_WORKSPACE="$PWD/.fleetworks"
export FLEETWORKS_UID="$(id -u)" FLEETWORKS_GID="$(id -g)"
export COMPOSE_PROJECT_NAME=fleetworks-local
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" init
```

Choose a distinct Compose project name and separate web and API ports for each concurrently running workspace.

## Client and web images

The service containers use the `runtime` image, which contains `argos-api-host`. Venture and each CLI have their own image, and the React Native web app uses the `web` image. A server build compiles the clients once and then copies each binary into its image.

```sh
docker buildx bake -f deploy/docker-bake.hcl venture fleetworks-brain web
docker run --rm venture:local --help
docker run --rm fleetworks-brain:local --help
```

Pull-request CI builds the selected targets for Linux AMD64. A full or nightly run also builds Linux ARM64. Registry publication is a separate release step.

The API uses `secrets/api-token` for clients. Private Brain, Tools, Control and Jobs use separate bearer files. The worker, Brain, brief worker and private execution service share the execution bearer. Venture creates `api-token`, `brain-token`, `tools-token`, `control-token`, `jobs-token` and `execution-token` under `secrets/` when it starts the stack. Bootstrap needs `secrets/storage-root-password` and `secrets/storage-password`. Use the existing files when restarting the same database. For a fresh managed database, generate each password file once.

```sh
(
  umask 077
  set -C
  for name in brain-token tools-token control-token jobs-token execution-token storage-root-password storage-password; do
    file="$FLEETWORKS_HOST_WORKSPACE/secrets/$name"
    [ -e "$file" ] || openssl rand -hex 32 >"$file" || exit
  done
)
```

The `FLEETWORKS_API_TOKEN_FILE`, `FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE`, and `FLEETWORKS_STORAGE_PASSWORD_FILE` settings accept absolute host-side paths. The runtime database password must be 32–256 characters using ASCII letters, digits, or `-._~`. Each container receives its required credentials as files under `/run/secrets`. Bootstrap gets the root and runtime passwords, the API gets the runtime password and the bearer tokens it needs to relay. Each private role gets the runtime password and its own bearer. The workers get the runtime password. For direct Compose use, `FLEETWORKS_BRAIN_TOKEN_FILE`, `FLEETWORKS_TOOLS_TOKEN_FILE`, `FLEETWORKS_CONTROL_TOKEN_FILE` and `FLEETWORKS_JOBS_TOKEN_FILE` override the private token paths. Venture uses the workspace path. The managed database reads the root password at startup. The web desktop receives the API token for its server-side HTTP and WebSocket relay.

## Managed disk database

```sh
mkdir -p "$FLEETWORKS_HOST_WORKSPACE/storage/surrealdb"
docker compose -f deploy/compose.yml --profile disk up --build --detach --wait
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" status
```

Open `http://127.0.0.1:3000` in a browser after readiness completes.

The `disk` profile uses the official SurrealDB 3.3.4 image pinned to its multi-platform digest. The database starts with authentication on the private network at `ws://surrealdb:8000`. It persists RocksDB data under `storage/surrealdb/` and syncs every transaction commit. Keep `FLEETWORKS_STORAGE_URL` unset or set to that origin when using the disk profile. `docker compose -f deploy/compose.yml --profile disk down` stops the stack and preserves its workspace data.

## Web development

From a source checkout, run `venture --workspace "$FLEETWORKS_HOST_WORKSPACE" up --dev`. Use `web --dev` to start the same stack with a Jobs or logs session. Venture records the development override for subsequent status, logs, and shutdown operations. Stop the workspace stack before switching between development and production.

The development image runs Next in development mode with pinned dependencies inside the container. Changes in `apps/web-desktop/app/` and the three shared `packages/*/src/` directories reload from read-only source mounts. Restart the stack to rebuild the image after changing dependency manifests, the relay, or Next configuration. Generated Next files stay in the container.

When running Compose directly, append `-f deploy/compose.dev.yml` after `-f deploy/compose.yml`. Keep both files in the command for the lifetime of the project.

Run `python3 deploy/verify.py --venture path/to/venture --dev` for a packaged development readiness and shutdown smoke check. The separate [browser development fixtures](../docs/wiki/code/react-ui-testing.md) exercise reload and live updates.

## Venture-managed host database

Set `FLEETWORKS_SURREAL_BIN` to an installed SurrealDB 3.3.4 executable or make `surreal` available on `PATH`. Venture launches the authenticated server, waits for readiness, and points the Compose services at the Docker host gateway.

```sh
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" up --storage host-disk
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" stop
```

Use `--storage host-memory` for a disposable database. Host disk persists under `storage/host-surrealdb/`. Both modes use the workspace storage credentials. `FLEETWORKS_HOST_DB_BIND` defaults to `0.0.0.0:8010` to reach Docker's host gateway. Set it to a specific reachable IPv4 interface and port to restrict the listener or run another workspace concurrently. Host SurrealDB diagnostics are in `logs/host-surreal.log`.

## External database

Set `FLEETWORKS_STORAGE_URL` to a reachable authenticated SurrealDB WebSocket origin. Use `wss://` for a remote TLS endpoint. For a server running on the Docker host, `host.docker.internal` is mapped to the host gateway. The external operator selects disk persistence and provides an existing root username and password for schema bootstrap.

```sh
export FLEETWORKS_STORAGE_URL=wss://surreal.example.test:8000
export FLEETWORKS_STORAGE_ROOT_USERNAME=root
export FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE=/absolute/path/to/root-password
docker compose -f deploy/compose.yml up --build --detach --wait
```

Both modes default `FLEETWORKS_STORAGE_NAMESPACE`, `FLEETWORKS_STORAGE_DATABASE`, and `FLEETWORKS_STORAGE_USERNAME` to `fleetworks`. Bootstrap creates the database-scoped runtime user if absent. Use the matching runtime password when reusing a provisioned database.

## Web, API, and image settings

Set `FLEETWORKS_WEB_PORT` to change the published web port. Compose sets `FLEETWORKS_WEB_ORIGIN` to the corresponding `http://127.0.0.1` origin. An override must use the exact loopback address and published port opened by the browser. The web relay reaches the API on the private `http://api:8080` origin, and its readiness probe checks that relay path.

Set `FLEETWORKS_API_PORT` to change the published API port and `FLEETWORKS_API_ORIGIN` to point Venture and the native clients at that port. `FLEETWORKS_API_BIND` defaults to `0.0.0.0` inside the container. The Compose host mapping always uses `127.0.0.1`. When overriding `FLEETWORKS_API_ALLOWED_HOSTS`, retain `api:8080` for the web relay and `127.0.0.1:<published API port>` for the direct authenticated readiness probe. Use `FLEETWORKS_API_ALLOWED_ORIGINS` for comma-separated origins if a browser needs direct API access. The web desktop uses its own origin by default.

The Dockerfile supports `linux/amd64` and `linux/arm64`. Build on the target architecture or use a Buildx builder with both platforms available. Select `--target web` for the production Next build on Node 24 and pnpm 12, `--target runtime` for the locked service host for the API, Brain, worker, and bootstrap roles, or `--target surrealdb` for the managed database image. The Rust runtime includes the current in-process NLP and classifier support. A separate Rust-BERT model/runtime bundle is outside this image.

The [Fleetworks Compose checks](../docs/wiki/code/ci.md#fleetworksyml--scoped-branch-checks) exercise the local images on native Linux ARM64 and AMD64 runners. Check the result for the source revision you plan to build.

### Start preloaded images

Load the images for every selected service into the local Docker engine, then use `venture up --no-build` or `venture web --no-build`. Venture passes `--no-build --pull never` to Compose, so the selected tags must already exist locally. The ordinary startup still builds images. Repeat `--no-build` on subsequent starts when using those images.

Set `FLEETWORKS_RUNTIME_IMAGE`, `FLEETWORKS_WEB_IMAGE`, and `FLEETWORKS_SURREAL_IMAGE` to select preloaded tags. With `--atlas`, also preload the pinned RabbitMQ image or set `FLEETWORKS_RABBITMQ_IMAGE` to its preloaded tag. For an artifact smoke, run `python3 deploy/verify.py --venture path/to/venture --no-build --image-ids path/to/service-images.json`. The JSON object maps each Compose service, including `bootstrap`, to its expected Docker image ID. The smoke checks the actual containers against those IDs before shutdown.

To exercise Venture-managed host storage with the same images, add `--storage host-memory --surreal /path/to/surreal` or `--storage host-disk --surreal /path/to/surreal`. Use the checksum-verified SurrealDB 3.3.4 executable. Each smoke uses a fresh workspace and a temporary host-gateway database port. It omits the Compose `surrealdb` service from the expected image IDs and verifies that a completed Tools result disappears in memory mode or survives in disk mode after a Venture stop and restart.

## Train the optional Natural classifier

After `venture up`, place the current CSV under workspace `datasets/` and run

```sh
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" train-legacy
```

Pass `--file datasets/another.csv` to select a different dataset, or configure `[datasets].articles` in workspace `config/workspace.toml`. Venture snapshots the selected bytes. The training worker checks that SHA-256 and reports it with the resulting model. Pass `--activate` to activate a synthetic Natural baseline explicitly. Atlas admission uses the separate Rust-BERT service.
