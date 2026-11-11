# Fleetworks service deployment

Docker Compose v2.20+ runs the web desktop, authenticated API, private services, workers, and database bootstrap. Web and API default to `127.0.0.1:3000` and `127.0.0.1:8080`. The [Atlas overlay](atlas-dispatch.md) adds AMQP dispatch, NLP, and the classifier.

## Prepare a workspace

Run from the repository root. Use an absolute workspace path that Docker can mount. The numeric UID and GID must be able to read its owner-only credential files and write its data directories.

```sh
export FLEETWORKS_HOST_WORKSPACE="$PWD/.fleetworks"
export FLEETWORKS_UID="$(id -u)" FLEETWORKS_GID="$(id -g)"
export COMPOSE_PROJECT_NAME=fleetworks-local
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" init
```

Venture derives a project name from the workspace path. `COMPOSE_PROJECT_NAME` selects the project for direct Compose commands. Concurrent workspaces need separate API, web, and host-service ports.

## Client and web images

`runtime` contains `argos-api-host`, `web` contains the React Native web app, and Venture and each CLI have separate images. Build from the repository root.

```sh
export VERSION="$(tr -d '[:space:]' < version.txt)"
docker buildx bake -f deploy/docker-bake.hcl venture fleetworks-brain web
docker run --rm "venture:${VERSION}" --help
docker run --rm "fleetworks-brain:${VERSION}" --help
```

Images use the workspace version from `version.txt` and a `:local` alias. Git release tags add `v`. [Build and release checks](../docs/wiki/code/ci.md)

## Credentials

Venture creates private credentials under `secrets/`. Direct Compose startup needs the following files in addition to the API token created by `venture init`. Reuse existing passwords when restarting a database.

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

Credential files must be owner-only. The runtime database password accepts 32–256 ASCII letters, digits, or `-._~`. Containers receive credentials under `/run/secrets` according to their roles.

Set `FLEETWORKS_API_TOKEN_FILE`, `FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE`, or `FLEETWORKS_STORAGE_PASSWORD_FILE` to override their host paths. Direct Compose also accepts `FLEETWORKS_{BRAIN,TOOLS,CONTROL,JOBS,EXECUTION}_TOKEN_FILE`. Venture provisions private-service tokens in the workspace.

## Managed disk database

```sh
mkdir -p "$FLEETWORKS_HOST_WORKSPACE/storage/surrealdb"
docker compose -f deploy/compose.yml --profile disk up --build --detach --wait
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" status
```

Open `http://127.0.0.1:3000` in a browser after readiness completes.

The `disk` profile runs the digest-pinned SurrealDB 3.3.4 image at `ws://surrealdb:8000`, with authenticated RocksDB storage under `storage/surrealdb/` and sync on every commit. Keep `FLEETWORKS_STORAGE_URL` unset or set to that origin. Use the same Compose command with `down` to stop services and retain data.

## Web development

Configure [local network services](#local-network-services), then run `venture --workspace "$FLEETWORKS_HOST_WORKSPACE" up --dev --storage local-net`. Use `web` in place of `up` to attach Jobs or logs. Venture remembers the development profile. Stop the stack before switching profiles.

Next runs in the container and reloads read-only mounts of `apps/web-desktop/app/` and shared `packages/*/src/`. Rebuild after changes to dependencies, the relay, or Next configuration. Generated files stay in the container.

When running Compose directly, append `-f deploy/compose.dev.yml` after `-f deploy/compose.yml`. Keep both files in the command for the lifetime of the project.

Run `python3 deploy/verify.py --venture path/to/venture --dev` for a packaged development readiness and shutdown smoke check. The separate [browser development fixtures](../docs/wiki/code/react-ui-testing.md) exercise reload and live updates.

## Choose a storage target

A new interactive `venture up` offers `local-net` by default, alongside `local-process` and `remote`. Repeat runs keep the recorded target. Stop the owned stack before changing targets. A new non-interactive launch defaults to `compose-disk`.

## Local network services

Use `local-net` for SurrealDB and RabbitMQ you run independently, such as containers publishing ports on your Docker host. Set the connection settings in `config/workspace.toml` or in the launch environment. File paths in workspace settings can be relative to the workspace.

```toml
[environment]
FLEETWORKS_STORAGE_URL = "ws://host.docker.internal:8000"
FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE = "secrets/local-net-surreal-root"
FLEETWORKS_BROKER_URL_FILE = "secrets/local-net-amqp-url"
```

Put the existing SurrealDB root password in its owner-only file. Override `FLEETWORKS_STORAGE_ROOT_USERNAME` if needed. For an existing runtime account, set `FLEETWORKS_STORAGE_PASSWORD_FILE` to its password file.

With `--atlas`, the owner-only broker file must contain an authenticated URL such as `amqp://worker:password@host.docker.internal:5672/fleetworks`. Percent-encode special characters in credentials and virtual hosts. Both endpoints must be reachable from Docker. `venture stop` leaves these services running.

## Venture-managed host processes

`venture up --storage local-process` starts authenticated SurrealDB and native RabbitMQ on the host. API services and workers run in Compose and connect through the host gateway. Add `--atlas` for AMQP dispatch.

Install SurrealDB 3.3.4 and `rabbitmq-server` with compatible Erlang on `PATH`. The local startup check used RabbitMQ 4.2.6 with Erlang/OTP 27.

| Service | Executable override | Bind setting and default | Data directory |
| --- | --- | --- | --- |
| SurrealDB | `FLEETWORKS_SURREAL_BIN` | `FLEETWORKS_HOST_DB_BIND=0.0.0.0:8010` | `storage/host-surrealdb/` |
| RabbitMQ | `FLEETWORKS_RABBITMQ_BIN` | `FLEETWORKS_HOST_BROKER_BIND=0.0.0.0:5672` | `storage/host-rabbitmq/` |

Bind addresses must be reachable from Docker. Keep the generated broker password across restarts. Logs are `logs/host-surreal.log` and `logs/host-rabbitmq.log`. `venture stop` stops both processes and retains data.

## Venture-managed host database

`--storage host-disk` and `--storage host-memory` manage SurrealDB with the executable, bind, credentials, and logs described above. Disk mode persists under `storage/host-surrealdb/`. Memory mode is disposable. Atlas uses its bundled RabbitMQ with either mode.

## External database

`--storage external` accepts a `wss://` origin, or `ws://host.docker.internal` for a host server. Venture checks `/health` before recording ownership. Supply the existing root credential for bootstrap.

```sh
export FLEETWORKS_STORAGE_URL=wss://surreal.example.test:8000
export FLEETWORKS_STORAGE_ROOT_USERNAME=root
export FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE=/absolute/path/to/root-password
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" up --storage external
```

All modes default `FLEETWORKS_STORAGE_NAMESPACE`, `FLEETWORKS_STORAGE_DATABASE`, and `FLEETWORKS_STORAGE_USERNAME` to `fleetworks`. Bootstrap provisions the runtime account. Reuse its matching password with an existing database.

## Web, API, and image settings

Set `FLEETWORKS_WEB_PORT` to change the published web port. Compose sets `FLEETWORKS_WEB_ORIGIN` to the corresponding `http://127.0.0.1` origin. An override must use the exact loopback address and published port opened by the browser. The web relay reaches the API on the private `http://api:8080` origin, and its readiness probe checks that relay path.

Set `FLEETWORKS_API_PORT` to change the published API port and `FLEETWORKS_API_ORIGIN` to point Venture and the native clients at that port. `FLEETWORKS_API_BIND` defaults to `0.0.0.0` inside the container. The Compose host mapping always uses `127.0.0.1`. When overriding `FLEETWORKS_API_ALLOWED_HOSTS`, retain `api:8080` for the web relay and `127.0.0.1:<published API port>` for the direct authenticated readiness probe. Use `FLEETWORKS_API_ALLOWED_ORIGINS` for comma-separated origins if a browser needs direct API access. The web desktop uses its own origin by default.

The Dockerfile supports `linux/amd64` and `linux/arm64`. Build on the target architecture or use a Buildx builder with both platforms available. Select `--target web` for the production Next build on Node 24 and pnpm 12, `--target runtime` for the locked service host for the API, Brain, worker, and bootstrap roles, or `--target surrealdb` for the managed database image. The Rust runtime includes the current in-process NLP and classifier support. A separate Rust-BERT model/runtime bundle is outside this image.

The [Fleetworks Compose checks](../docs/wiki/code/ci.md#fleetworksyml--scoped-branch-checks) exercise the local images on native Linux ARM64 and AMD64 runners. Check the result for the source revision you plan to build.

### Start preloaded images

Load the images for every selected service into the local Docker engine, then use `venture up --no-build` or `venture web --no-build`. Venture passes `--no-build --pull never` to Compose, so the selected tags must already exist locally. The ordinary startup still builds images. Repeat `--no-build` on subsequent starts when using those images.

Set `FLEETWORKS_RUNTIME_IMAGE`, `FLEETWORKS_WEB_IMAGE`, and `FLEETWORKS_SURREAL_IMAGE` for preloaded tags. Atlas needs `FLEETWORKS_RABBITMQ_IMAGE` when using the bundled broker. For an artifact smoke, run `python3 deploy/verify.py --venture path/to/venture --no-build --image-ids path/to/service-images.json`. The JSON maps every selected Compose service, including `bootstrap`, to its expected image ID.

To exercise Venture-managed host storage with the same images, add `--storage host-memory --surreal /path/to/surreal` or `--storage host-disk --surreal /path/to/surreal`. Use the checksum-verified SurrealDB 3.3.4 executable. Each smoke uses a fresh workspace and a temporary host-gateway database port. It omits the Compose `surrealdb` service from the expected image IDs and verifies that a completed Tools result disappears in memory mode or survives in disk mode after a Venture stop and restart.

## Train the optional Natural classifier

After `venture up`, place the current CSV under workspace `datasets/` and run

```sh
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" train-legacy
```

Pass `--file datasets/another.csv` to select a different dataset, or configure `[datasets].articles` in workspace `config/workspace.toml`. Venture snapshots the selected bytes. The training worker checks that SHA-256 and reports it with the resulting model. Pass `--activate` to activate a synthetic Natural baseline explicitly. Atlas admission uses the separate Rust-BERT service.
