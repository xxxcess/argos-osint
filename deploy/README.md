# Fleetworks service deployment

This stack runs the production web desktop, authenticated API, background worker pools, and one-shot database bootstrap. Docker Compose v2.20 or newer is required. The web desktop and API publish `127.0.0.1:3000` and `127.0.0.1:8080` by default. Database connections stay inside the Compose network for the managed profile. RabbitMQ dispatch, standalone engine services, and the development web overlay are pending their runtime integration.

## Prepare a workspace

Run from the repository root. Use an absolute workspace path that Docker can mount. The numeric UID and GID must be able to read its owner-only credential files and write its data directories.

```sh
export FLEETWORKS_HOST_WORKSPACE="$PWD/.fleetworks"
export FLEETWORKS_UID="$(id -u)" FLEETWORKS_GID="$(id -g)"
export COMPOSE_PROJECT_NAME=fleetworks-local
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" init
```

Choose a distinct Compose project name and separate web and API ports for each concurrently running workspace.

The API uses `secrets/api-token`. Bootstrap needs `secrets/storage-root-password` and `secrets/storage-password`. Use the existing files when restarting the same database. For a fresh managed database, generate each password file once.

```sh
(
  umask 077
  set -C
  for name in storage-root-password storage-password; do
    file="$FLEETWORKS_HOST_WORKSPACE/secrets/$name"
    [ -e "$file" ] || openssl rand -hex 32 >"$file" || exit
  done
)
```

The `FLEETWORKS_API_TOKEN_FILE`, `FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE`, and `FLEETWORKS_STORAGE_PASSWORD_FILE` settings accept absolute host-side paths. The runtime database password must be 32–256 characters using ASCII letters, digits, or `-._~`. Each container receives its required credentials as files under `/run/secrets`. Bootstrap gets the root and runtime passwords, the API gets the runtime password and bearer token, and the worker gets the runtime password. The managed database reads the root password at startup. The web desktop receives the API token for its server-side HTTP and WebSocket relay.

## Managed disk database

```sh
mkdir -p "$FLEETWORKS_HOST_WORKSPACE/storage/surrealdb"
docker compose -f deploy/compose.yml --profile disk up --build --detach --wait
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" status
```

Open `http://127.0.0.1:3000` in a browser after readiness completes.

The `disk` profile uses the official SurrealDB 3.3.4 image pinned to its multi-platform digest. The database starts with authentication on the private network at `ws://surrealdb:8000`. It persists RocksDB data under `storage/surrealdb/` and syncs every transaction commit. Keep `FLEETWORKS_STORAGE_URL` unset or set to that origin when using the disk profile. `docker compose -f deploy/compose.yml --profile disk down` stops the stack and preserves its workspace data.

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

The Dockerfile supports `linux/amd64` and `linux/arm64`. Build on the target architecture or use a Buildx builder with both platforms available. Select `--target web` for the production Next build on Node 24 and pnpm 12, `--target runtime` for the locked API, worker, and bootstrap binary, or `--target surrealdb` for the managed database image. The Rust runtime includes the current in-process NLP and classifier support. A separate Rust-BERT model/runtime bundle is outside this image.
