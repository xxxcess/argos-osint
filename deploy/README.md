# Fleetworks service deployment

Docker Compose v2.20+ runs the web desktop, authenticated API, private services, workers, and database bootstrap. Web and API default to `127.0.0.1:3000` and `127.0.0.1:8080`. The [Atlas overlay](atlas-dispatch.md) adds the Atlas dispatch worker, NLP, and the classifier.

On macOS, Venture also searches the Docker Desktop CLI directories under `/Applications` and `~/Applications` when running Docker.

Rust image builds use one Cargo job and 256 core code-generation units to limit memory use. Local Compose builds use core optimization level 1. Set `FLEETWORKS_CORE_OPT_LEVEL=3` for full optimization with more build memory. Direct Docker builds default to level 3 and accept these settings as build arguments. If memory is exhausted, increase Docker Desktop's memory under Settings → Resources → Memory and retry. BuildKit retains completed dependencies.

## Choose a development loop

Venture and the CLIs run on the host; service roles, web and the managed database run from the `runtime`, `web` and `surrealdb` Dockerfile targets. Rebuild only the image whose code changed: CLI changes need a host build only, service changes rebuild `runtime`, web source edits reload in the [development container](#web-development), and Dockerfile or dependency changes rebuild the affected target with [Buildx bake](#client-and-web-images). See the [contributor loop guide](../docs/wiki/code/contributing.md#development-loops-local-cli-and-docker) for the full order of checks. Use a separate workspace, `COMPOSE_PROJECT_NAME` and ports for each concurrent stack.

[![Dockerfile targets and packaged image loop](../docs/diagrams/dev-loop-images.svg)](../docs/diagrams/dev-loop-images.html)

## Isolated native backend review

Use the standalone [native review recipe](compose.native-review.yml) for the Rune app's independently running backend. Its `backend` Bake group builds only `runtime`, digest-pinned `surrealdb`, and `search-index`. The search image selects only `fleetworks-search-index` and shared contracts; it compiles no core inference, terminal, native view or Next.js bundle. The existing operator/web targets below remain available.

Provision a **new** task-owned deployment directory, separate from every native workspace and existing stack. The helper creates private files and directories only; an existing review name fails without replacing credentials or data. Choose an unused API port. It creates no browser service or published database/search port.

```sh
export REVIEW_NAME=rune-search-review
export PLATFORM="$(python3 scripts/fleetworks_verify.py platform --format docker)"
python3 deploy/native_review.py init --name "$REVIEW_NAME" --api-port 18091 --platform "$PLATFORM"
export REVIEW_WORKSPACE="$(python3 deploy/native_review.py value --name "$REVIEW_NAME" --field workspace)"
export IMAGE_TAG="$(python3 deploy/native_review.py value --name "$REVIEW_NAME" --field image_tag)"
export REVIEW_PROJECT="$(python3 deploy/native_review.py value --name "$REVIEW_NAME" --field project)"
export VERSION="$(tr -d '[:space:]' < version.txt)"
```

The workspace lives under `.agent-scratch/native-review/<name>`. Its FORMAT 1 `secrets/gateway-credentials.json` references separate owner-only token files for Native, WebRelay and InternalService purposes. Native may register/maintain; WebRelay may resolve existing browser context and cannot register or maintain; the internal health credential has read permission only. Every private runtime role receives the same persisted `FLEETWORKS_ACCESS_SCOPE_ID` and `FLEETWORKS_ACCESS_EPOCH`. Search read/write tokens differ, and only search plus `native-worker` receive the writer token. Keep these files and the unique search source epoch across restarts. Startup never rotates an epoch or resets a volume. Tokens stay out of command arguments, output and build artifacts; do not print or source `compose.env`.

Inspect the selected Docker context, daemon and builder before building. A remote context or a Linux architecture different from `$PLATFORM` needs an explicit operator decision; do not silently emulate. Use a native AMD64/ARM64 builder for that platform. These single-platform builds load review tags and never overwrite `:local` or version aliases:

```sh
docker context show
docker info --format '{{.OSType}}/{{.Architecture}}'
docker buildx inspect
docker buildx bake -f deploy/docker-bake.hcl --print backend
docker buildx bake -f deploy/docker-bake.hcl --load \
  --metadata-file "$REVIEW_WORKSPACE/build-metadata.json" backend
docker image inspect "fleetworks-runtime:$IMAGE_TAG" "fleetworks-surrealdb:$IMAGE_TAG" \
  "fleetworks-search-index:$IMAGE_TAG" \
  --format '{{.Id}} {{.Os}}/{{.Architecture}} {{json .RepoTags}}' \
  > "$REVIEW_WORKSPACE/image-identities.txt"
```

Run a separate fresh review on `PLATFORM=linux/amd64` and `PLATFORM=linux/arm64` to collect both native Linux results. Preserve each platform's Buildx metadata and exact inspected IDs alongside its source revision and dirty status. Build completion is distinct from startup or integration evidence. A narrow runtime-only iteration can use `docker buildx build --platform "$PLATFORM" -f deploy/Dockerfile --target runtime --build-arg FLEETWORKS_CORE_OPT_LEVEL=1 --tag "fleetworks-runtime:$IMAGE_TAG" --load .`; it does not produce the other required images or start anything.

Validate and start the exact preloaded tags in a second operator action. Keep the same env file, standalone Compose file and project on every command. No native folder is mounted here.

```sh
docker compose --env-file "$REVIEW_WORKSPACE/compose.env" -f deploy/compose.native-review.yml \
  --project-name "$REVIEW_PROJECT" config --quiet
docker compose --env-file "$REVIEW_WORKSPACE/compose.env" -f deploy/compose.native-review.yml \
  --project-name "$REVIEW_PROJECT" up --no-build --pull never --detach --wait
docker compose --env-file "$REVIEW_WORKSPACE/compose.env" -f deploy/compose.native-review.yml \
  --project-name "$REVIEW_PROJECT" ps
```

Bootstrap provisions schema/runtime credentials and exports the retained database identity to `deployment/deployment-id`; search reads it from a read-only mount. SurrealDB, Jobs, and the search index use separate persistent owned directories. Jobs alone opens its SQLite/dispatch files. `native-worker` consumes durable seed/train/tools jobs through Jobs and pumps the database projection outbox into search. API and worker share `/workspace/staged-datasets` within this backend directory. Container CPU, memory and process limits are explicit; search uses one writer, one merge worker, 64 MiB writer memory and four query slots. These are bounded review defaults, not measured capacity guarantees.

Only the loopback gateway is published, at `http://127.0.0.1:18091` for the example. In the native connection editor choose that gateway, the protected `secrets/native-token` file, and a different app-owned local review folder. Register it through the app's workspace action. The generated web-relay token is reserved for a separately configured browser relay; this stack starts no web frontend. Remote access needs an independently configured HTTPS ingress and allowed-host policy.

Search process health authenticates `/internal/v1/actors/status`; an empty generation can be healthy while discovery remains `not_built`/`indexing`. Seed completion and reader-visible index completion are separate. Search failure does not gate gateway or seed admission startup. Source/generation mismatch, corruption and writer conflicts stay visible and preserve data; there is no automatic delete/rebuild. Coordinated shadow rebuild, pressure adaptation and repair administration remain pending, so retain the old serving generation and diagnose before changing any directory or source epoch.

Atlas/NLP/Rust-BERT and optional web deployment remain separate operator flows below and in [Atlas dispatch](atlas-dispatch.md). Stage a hash-verified, architecture-matching **Linux** model/native-library bundle in the backend's `models/atlas-bert/` before explicitly selecting those services; never copy host macOS libraries or treat Natural training as Rust-BERT inference evidence. This core review stack makes no real-checkpoint inference claim.

Rebuilding a tag does not replace a running consumer. Explicitly recreate the selected services after a successful build. Inspect logs and stop this review with the same project; retain credentials, source identity and data:

```sh
docker compose --env-file "$REVIEW_WORKSPACE/compose.env" -f deploy/compose.native-review.yml \
  --project-name "$REVIEW_PROJECT" logs --tail 100 api native-worker search-index
docker compose --env-file "$REVIEW_WORKSPACE/compose.env" -f deploy/compose.native-review.yml \
  --project-name "$REVIEW_PROJECT" down
```

Do not use `down -v` or global image/cache cleanup as routine review cleanup. Docker build/start, both Linux architectures, real dataset operations, recovery and inference evidence must be recorded when actually exercised; source/config checks alone do not satisfy those gates.

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

`venture init` provisions a FORMAT 1 gateway manifest under `secrets/gateway-credentials.json`, new distinct `native-api-token`, `web-relay-token` and read-only `operator-api-token` files, and stable scope/access epoch metadata in `config/gateway-authority.json`. Existing `api-token` bytes are preserved and excluded from the manifest. They acquire no new authority. Repeated initialization verifies existing provisioning instead of rotating it. Stop a stack with legacy ownership before restarting it with the current gateway configuration; retained ownership decoding still permits that shutdown.

Use `secrets/native-api-token` with the native GUI's normal registration flow. The web container receives only `web-relay-token`; a browser must enter a registered Connect ID and receives a protected server session. `operator-api-token` supports gateway readiness and global diagnostic reads, including `venture status`; it cannot register, mutate or browse workstation activity without authority. Terminal presentation/mutation retirement remains a separate gate. No caller-assigned body origin or old shared token grants workstation rights.

`venture up` supplies the persisted scope/epoch and manifest paths to Compose. For direct Compose, load the same **non-secret** authority metadata explicitly after `venture init`:

```sh
export FLEETWORKS_ACCESS_SCOPE_ID="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["scope_id"])' "$FLEETWORKS_HOST_WORKSPACE/config/gateway-authority.json")"
export FLEETWORKS_ACCESS_EPOCH="$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["access_epoch"])' "$FLEETWORKS_HOST_WORKSPACE/config/gateway-authority.json")"
```

Direct Compose startup also needs the following role credentials. Reuse existing passwords when restarting a database.

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

Direct Compose accepts `FLEETWORKS_API_CREDENTIALS_FILE`, `FLEETWORKS_NATIVE_TOKEN_FILE`, `FLEETWORKS_WEB_RELAY_TOKEN_FILE` and `FLEETWORKS_OPERATOR_TOKEN_FILE` for an explicitly provisioned matching manifest and protected files. It also accepts `FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE`, `FLEETWORKS_STORAGE_PASSWORD_FILE` and `FLEETWORKS_{BRAIN,TOOLS,CONTROL,JOBS,EXECUTION}_TOKEN_FILE`. Venture provisions private-service tokens separately and rejects a legacy `FLEETWORKS_API_TOKEN_FILE` override as gateway authority. No current service mounts the retained old shared API bearer.

Keep the deployment's persisted `FLEETWORKS_ACCESS_SCOPE_ID` and `FLEETWORKS_ACCESS_EPOCH` aligned with its purpose manifest. Conflicting environment values fail instead of replacing retained authority. There are no fixed authority defaults or automatic rotations. Empty values allow inspection/shutdown of old Compose ownership only; protected service startup rejects them. The standalone native review recipe uses its own independently provisioned authority pair and token filenames.

## Managed disk database

```sh
mkdir -p "$FLEETWORKS_HOST_WORKSPACE/storage/surrealdb" "$FLEETWORKS_HOST_WORKSPACE/storage/jobs"
docker compose -f deploy/compose.yml --profile disk up --build --detach --wait
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" status
```

Open `http://127.0.0.1:3000` after process readiness, then enter a Connect ID from the registered native workspace. Unconnected application/data requests remain denied. The web process health check loads the public `/connect` page; authenticated relay readiness is verified only after a real session is established.

The `disk` profile runs the digest-pinned SurrealDB 3.3.4 image at `ws://surrealdb:8000`, with authenticated RocksDB storage under `storage/surrealdb/` and sync on every commit. Keep `FLEETWORKS_STORAGE_URL` unset or set to that origin. Use the same Compose command with `down` to stop services and retain data.

## Operational storage

The Jobs process alone owns `storage/jobs/`: SQLite operational logs in `jobs.sqlite3` and the embedded RocksDB dispatch queue in `queue/`. Other services connect through authenticated Jobs HTTP. Preserve the entire directory across restarts; do not open it from another process or mount it into workers. No RabbitMQ server, Erlang installation or broker credential is required.

Domain job/task transitions remain fenced in SurrealDB during the SQLite scheduler cutover. The embedded queue transports dispatches; it does not independently authorize execution. For a workspace previously using RabbitMQ, drain existing Atlas jobs with the previous stack, then stop it before upgrading. Venture retains shutdown compatibility for legacy owned processes and preserves their files. Pending legacy dispatches fail closed rather than silently disappearing. See [Atlas dispatch](atlas-dispatch.md) and the [job persistence target and rollout phases](../docs/wiki/code/jobs-persistence.md).

## Web development

Configure [local network services](#local-network-services), then run `venture --workspace "$FLEETWORKS_HOST_WORKSPACE" up --dev --storage local-net`. Use `web` in place of `up` to attach Jobs or logs. Venture remembers the development profile. Stop the stack before switching profiles.

Next runs in the container and reloads read-only mounts of `apps/web-desktop/app/` and shared `packages/*/src/`. Rebuild after changes to dependencies, the relay, or Next configuration. Generated files stay in the container.

When running Compose directly, append `-f deploy/compose.dev.yml` after `-f deploy/compose.yml`. Keep both files in the command for the lifetime of the project.

Run `python3 deploy/verify.py --venture path/to/venture --dev` for a packaged development readiness and shutdown smoke check. The separate [browser development fixtures](../docs/wiki/code/react-ui-testing.md) exercise reload and live updates.

## Choose a storage target

A new interactive `venture up` offers `local-net` by default, alongside `local-process` and `remote`. Repeat runs keep the recorded target. Stop the owned stack before changing targets. A new non-interactive launch defaults to `compose-disk`.

## Local network services

Use `local-net` for SurrealDB you run independently, such as containers publishing ports on your Docker host. Set the connection settings in `config/workspace.toml` or in the launch environment. File paths in workspace settings can be relative to the workspace.

```toml
[environment]
FLEETWORKS_STORAGE_URL = "ws://host.docker.internal:8000"
FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE = "secrets/local-net-surreal-root"
```

Put the existing SurrealDB root password in its owner-only file. Override `FLEETWORKS_STORAGE_ROOT_USERNAME` if needed. For an existing runtime account, set `FLEETWORKS_STORAGE_PASSWORD_FILE` to its password file.

The SurrealDB endpoint must be reachable from Docker. `venture stop` leaves that external service running. Atlas dispatch uses the Jobs service’s embedded disk queue; no RabbitMQ configuration is required.

## Venture-managed host processes

`venture up --storage local-process` starts authenticated SurrealDB on the host. API services and workers run in Compose and connect through the host gateway. Add `--atlas` for the Atlas worker and private NLP/classifier services.

Install SurrealDB 3.3.4 on `PATH`, or set its executable override below.

| Service | Executable override | Bind setting and default | Data directory |
| --- | --- | --- | --- |
| SurrealDB | `FLEETWORKS_SURREAL_BIN` | `FLEETWORKS_HOST_DB_BIND=0.0.0.0:8010` | `storage/host-surrealdb/` |

The bind address must be reachable from Docker. Logs are in `logs/host-surreal.log`. `venture stop` stops the owned process and retains data.

## Venture-managed host database

`--storage host-disk` and `--storage host-memory` manage SurrealDB with the executable, bind, credentials, and logs described above. Disk mode persists under `storage/host-surrealdb/`. Memory mode is disposable. Atlas uses the Jobs service’s embedded disk queue with either mode.

## External database

`--storage external` accepts a `wss://` origin, or `ws://host.docker.internal` for a host server. Venture checks `/health` before recording ownership. Supply the existing root credential for bootstrap.

```sh
export FLEETWORKS_STORAGE_URL=wss://surreal.example.test:8000
export FLEETWORKS_STORAGE_ROOT_USERNAME=root
export FLEETWORKS_STORAGE_ROOT_PASSWORD_FILE=/absolute/path/to/root-password
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" up --storage external
```

All modes default `FLEETWORKS_STORAGE_NAMESPACE`, `FLEETWORKS_STORAGE_DATABASE`, and `FLEETWORKS_STORAGE_USERNAME` to `fleetworks`. Bootstrap provisions the runtime account. Reuse its matching password with an existing database.

Bootstrap preserves retained data when upgrading pool metadata. Its first configuration update initializes a missing `transport_revision` to `0`, preserving existing revisions and transport markers. Update the runtime image if bootstrap reports `Expected int but found NONE` for this field; resetting the database is unnecessary.

## Web, API, and image settings

Set `FLEETWORKS_WEB_PORT` to change the published web port. Compose sets `FLEETWORKS_WEB_ORIGIN` to the corresponding `http://127.0.0.1` origin. An override must use the exact loopback address and published port opened by the browser. The web relay reaches the API on private `http://api:8080` using its dedicated WebRelay credential; a public process probe is separate from session-protected relay reads.

Set `FLEETWORKS_API_PORT` to change the published API port and `FLEETWORKS_API_ORIGIN` to point Venture and the native clients at that port. `FLEETWORKS_API_BIND` defaults to `0.0.0.0` inside the container. The Compose host mapping always uses `127.0.0.1`. When overriding `FLEETWORKS_API_ALLOWED_HOSTS`, retain `api:8080` for the web relay and `127.0.0.1:<published API port>` for the direct authenticated readiness probe. Use `FLEETWORKS_API_ALLOWED_ORIGINS` for comma-separated origins if a browser needs direct API access. The web desktop uses its own origin by default.

The Dockerfile supports `linux/amd64` and `linux/arm64`. Build on the target architecture or use a Buildx builder with both platforms available. Select `--target web` for the production Next build on Node 24 and pnpm 12, `--target runtime` for the locked service host for the API, Brain, worker, and bootstrap roles, or `--target surrealdb` for the managed database image. The Rust runtime includes the current in-process NLP and classifier support. A separate Rust-BERT model/runtime bundle is outside this image.

The [Fleetworks Compose checks](../docs/wiki/code/ci.md#fleetworksyml--scoped-branch-checks) exercise the local images on native Linux ARM64 and AMD64 runners. Check the result for the source revision you plan to build.

### Start preloaded images

Load the images for every selected service into the local Docker engine, then use `venture up --no-build` or `venture web --no-build`. Venture passes `--no-build --pull never` to Compose, so the selected tags must already exist locally. The ordinary startup still builds images. Repeat `--no-build` on subsequent starts when using those images.

Set `FLEETWORKS_RUNTIME_IMAGE`, `FLEETWORKS_WEB_IMAGE`, and `FLEETWORKS_SURREAL_IMAGE` for preloaded tags. For an artifact smoke, run `python3 deploy/verify.py --venture path/to/venture --no-build --image-ids path/to/service-images.json`. The JSON maps every selected Compose service, including `bootstrap`, to its expected image ID.

The smoke creates a contained synthetic child in its own temporary home, performs genuine Native ensure/context, checks missing-proof and other-workstation denial, submits durable Tools work and reads its attributed activity, then connects/logs out through the normal browser cookie flow. `native-worker` consumes the actual durable Tools/setup queues and must map to the loaded runtime image. These are backend integration fixtures, distinct from production `NativeContainer` filesystem tests, installed-app GUI checks and retired terminal presentation. The retained `--train-legacy-dataset` spelling now stages the immutable CSV through the gateway and runs registered Natural training twice while retaining prior activation; it does not call the old contextless activation command or claim Rust-BERT inference.

To exercise Venture-managed host storage with the same images, add `--storage host-memory --surreal /path/to/surreal` or `--storage host-disk --surreal /path/to/surreal`. Use the checksum-verified SurrealDB 3.3.4 executable. Each smoke uses a fresh workspace and a temporary host-gateway database port. It omits the Compose `surrealdb` service from the expected image IDs and verifies that a completed Tools result disappears in memory mode or survives in disk mode after a Venture stop and restart.

## Train the optional Natural classifier

After `venture up`, place the current CSV under workspace `datasets/` and run

```sh
venture --workspace "$FLEETWORKS_HOST_WORKSPACE" train-legacy
```

Pass `--file datasets/another.csv` to select a different dataset, or configure `[datasets].articles` in workspace `config/workspace.toml`. Venture snapshots the selected bytes. The training worker checks that SHA-256 and reports it with the resulting model. Pass `--activate` to activate a synthetic Natural baseline explicitly. Atlas admission uses the separate Rust-BERT service.

### Jobs SQLite storage

Jobs exclusively owns `<workspace>/storage/jobs/jobs.sqlite3`, mounted at `/var/lib/fleetworks-jobs` with the workspace owner's UID. Venture creates this directory; direct Compose users must create `storage/jobs/` first. Keep it on local storage. Other services send operational logs through the private Jobs bearer connection; they never open its database. Preserve the directory across container replacement. Use SQLite's backup API or stop Jobs before copying its database and WAL files.

The current integration moves standalone logs. Scheduling, attempts, recovery and telemetry remain in SurrealDB until the drain-first scheduler migration and publication fences are complete. Historical SurrealDB logs are retained there.
