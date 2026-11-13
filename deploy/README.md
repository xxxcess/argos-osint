# Fleetworks service deployment

Docker Compose v2.20+ runs the authenticated API, private services, workers and database bootstrap. Provision an explicit loopback API port and, when selecting Intel Web, a separate Web port. The examples use `18091` and `18092`. The [Atlas overlay](atlas-dispatch.md) adds the Atlas dispatch worker, NLP and classifier.

Use the Docker CLI from the selected operator shell and inspect the active context before building or starting services.

Rust image builds use one Cargo job and 256 core code-generation units to limit memory use. The runtime Dockerfile defaults to core optimization level 3; pass `--build-arg FLEETWORKS_CORE_OPT_LEVEL=1` for a faster local runtime build as shown below. If memory is exhausted, increase Docker Desktop's memory under Settings → Resources → Memory and retry. BuildKit retains completed dependencies.

## Choose a development loop

The Rune app builds on macOS; Linux services, web and the database use the `runtime`, `web`, `surrealdb` and `search-index` Dockerfile targets. Rebuild only the affected image. Native view changes use the host app loop, and browser source changes use the separate Next development loop. See the [contributor loop guide](../docs/wiki/code/contributing.md#development-loops-local-cli-and-docker) for the full order of checks. Use a separate workspace, `COMPOSE_PROJECT_NAME` and ports for each concurrent stack.

[![Dockerfile targets and packaged image loop](../docs/diagrams/dev-loop-images.svg)](../docs/diagrams/dev-loop-images.html)

## Isolated native backend review

Use the standalone [native review recipe](compose.native-review.yml) for the Rune app's independently running backend. Its `backend` Bake group builds only `runtime`, digest-pinned `surrealdb`, and `search-index`. The search image selects only `fleetworks-search-index` and shared contracts; it compiles no core inference, terminal, native view or Next.js bundle. Web deployment is an explicit separate selection. No launcher or terminal-client image is built.

Provision a **new** task-owned deployment directory, separate from every native workspace and existing stack. The helper creates private files and directories only; an existing review name fails without replacing credentials or data. Choose an unused API port. It creates no browser service or published database/search port.

```sh
export REVIEW_NAME=native-local
export PLATFORM="$(python3 scripts/fleetworks_verify.py platform --format docker)"
python3 deploy/native_review.py init --name "$REVIEW_NAME" --api-port 18091 --platform "$PLATFORM" --root-env
export REVIEW_WORKSPACE="$(python3 deploy/native_review.py value --name "$REVIEW_NAME" --field workspace)"
export IMAGE_TAG="$(python3 deploy/native_review.py value --name "$REVIEW_NAME" --field image_tag)"
export REVIEW_PROJECT="$(python3 deploy/native_review.py value --name "$REVIEW_NAME" --field project)"
export VERSION="$(tr -d '[:space:]' < version.txt)"
```

The editable `compose.env` lives at the repository root, is mode 0600 and is ignored by Git. `--root-env` refuses an existing root file. The workspace's generated `compose.env` points to the root file, so older commands read the same settings, including after an editor atomically replaces it. Initialization runs once; use the existing file for subsequent starts. For concurrent reviews or separate platform results, omit `--root-env` and use that review's `$REVIEW_WORKSPACE/compose.env` in every Compose command below.

The workspace lives under `.agent-scratch/native-review/<name>`. Its FORMAT 1 `secrets/gateway-credentials.json` references separate owner-only token files for Native, Operator, WebRelay and InternalService purposes. Native may register/maintain; WebRelay may resolve existing browser context and cannot register or maintain; the internal health credential has read permission only. Every private runtime role receives the same persisted `FLEETWORKS_ACCESS_SCOPE_ID` and `FLEETWORKS_ACCESS_EPOCH`. Search read/write tokens differ, and only search plus `native-worker` receive the writer token. Keep these files and the unique search source epoch across restarts. Startup never rotates an epoch or resets a volume. Tokens stay out of command arguments, output and build artifacts; do not print or source `compose.env`.

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
docker compose --env-file compose.env -f deploy/compose.native-review.yml \
  --project-name "$REVIEW_PROJECT" config --quiet
docker compose --env-file compose.env -f deploy/compose.native-review.yml \
  --project-name "$REVIEW_PROJECT" up --no-build --pull never --detach --wait
docker compose --env-file compose.env -f deploy/compose.native-review.yml \
  --project-name "$REVIEW_PROJECT" ps
```

Bootstrap provisions schema/runtime credentials and exports the retained database identity to `deployment/deployment-id`; search reads it from a read-only mount. SurrealDB, Jobs, and the search index use separate persistent owned directories. Jobs alone opens its SQLite/dispatch files. `native-worker` consumes durable seed/train/tools jobs through Jobs and pumps the database projection outbox into search. API and worker share `/workspace/staged-datasets` within this backend directory. Container CPU, memory and process limits are explicit; search uses one writer, one merge worker, 64 MiB writer memory and four query slots. These are bounded review defaults, not measured capacity guarantees.

Only the loopback gateway is published, at `http://127.0.0.1:18091` for the example. Enter that URL in native Setup Gateway and choose Connect; approve its displayed request ID/code with the separate Operator credential. The app authenticates and stores access in macOS Keychain before registration. No native-user token file is needed. The helper prints the approval command. [Pairing/recovery](../docs/wiki/code/native-connections.md#pairing-and-keychain-agreed-target)

This stack starts no web frontend. WebRelay remains separately configured; remote access requires independently configured HTTPS ingress and allowed-host policy. Explicit operator/headless clients retain token-file mode. [Verification status](../.planning/2026-11-12-native-pairing-implementation/acceptance.md) separates source/test evidence from actual backend and live Keychain journeys.

Search process health authenticates `/internal/v1/actors/status`; an empty generation can be healthy while discovery remains `not_built`/`indexing`. Seed completion and reader-visible index completion are separate. Search failure does not gate gateway or seed admission startup. Source/generation mismatch, corruption and writer conflicts stay visible and preserve data; there is no automatic delete/rebuild. Explicit maintenance reports rebuild/pressure state; retain the old serving generation and diagnose before changing any directory or source epoch.

Atlas/NLP/Rust-BERT and optional web deployment remain separate operator flows; see [Atlas dispatch](atlas-dispatch.md). Stage a hash-verified, architecture-matching **Linux** model/native-library bundle in the backend's `models/atlas-bert/` before explicitly selecting those services; never copy host macOS libraries or treat Natural training as Rust-BERT inference evidence. This core review stack makes no real-checkpoint inference claim.

Rebuilding a tag does not replace a running consumer. Explicitly recreate the selected services after a successful build. Inspect logs and stop this review with the same project; retain credentials, source identity and data:

```sh
docker compose --env-file compose.env -f deploy/compose.native-review.yml \
  --project-name "$REVIEW_PROJECT" logs --tail 100 api native-worker search-index
docker compose --env-file compose.env -f deploy/compose.native-review.yml \
  --project-name "$REVIEW_PROJECT" down
```

Do not use `down -v` or global image/cache cleanup as routine review cleanup. Docker build/start, both Linux architectures, real dataset operations, recovery and inference evidence must be recorded when actually exercised; source/config checks alone do not satisfy those gates.

## Credentials

Fresh initialization creates a FORMAT 1 gateway manifest and distinct private Native, Operator, WebRelay and internal-health token files. `secrets/operator-token` is create-exclusive mode 0600; its `operator-review` entry shares this deployment’s scope/epoch and Native grant set, excluding ResolveWebSession. Compose mounts it as `gateway_operator_token` only in the API gateway. Operator approval requires ReadBackend, RegisterWorkstation and ManageConfiguration; grants are server-derived. [Approval/revocation command](../docs/wiki/code/native-connections.md#operator-approval-and-revocation)

Existing deployment directories are never backfilled or overwritten. Their operator must provision an independent private Operator token and matching manifest entry/mount, then restart the gateway using its exact existing env file/project selection. Never relabel/reuse Native or WebRelay tokens. Preserve scope/epoch, credentials, source identity and data; changing a mounted file alone does not prove policy reload.

Normal native clients pair into Keychain. `secrets/native-token` remains for explicit file-based tooling; Next uses `secrets/web-relay-token` independently. WebRelay cannot register/approve; health and old shared bearers grant no mutation rights. Private-service/storage/search credentials retain their separate owners. Schema 10 requires coordinated bootstrap and compatible readers; preserve existing identity/origins and static/headless credentials.

## Operational storage

Jobs alone opens `storage/jobs/`: SQLite operational logs and its embedded RocksDB dispatch queue. Other services use authenticated private Jobs HTTP. Preserve the entire directory across restarts. SurrealDB retains domain records and current scheduler state; the separate [SQLite scheduler cutover](../docs/wiki/code/jobs-persistence.md) remains incomplete. Search has its own owned index directory and database/source identity.

Setup starts with a fresh deployment directory and new operational stores. There are no pre-existing jobs or queue data to migrate. Subsequent starts retain the deployment's credentials, configuration, epochs and acknowledged work; ordinary startup never resets them.

## Web and development

Intel Web is separately deployed with the `web` Docker target or its Next development server. Provision its fixed internal gateway origin, separate WebRelay token-file reference, public advertised origin and Origin/Host policy. Remote browser access requires HTTPS at the public ingress; internal upstream addresses remain private. The browser starts at `/connect` and needs an existing registered Connect ID. Native may close while the browser and accepted backend work continue.

For a new browser-enabled deployment, add `--web-port 18092` to the provisioning command above before creating the directory. The helper rejects an existing name and records the distinct explicit Web origin; do not rerun initialization to mutate an existing deployment. Build the Web image under the same isolated tag and select both base and overlay:

```sh
docker buildx bake -f deploy/docker-bake.hcl --load backend web
docker compose --env-file compose.env \
  -f deploy/compose.native-review.yml -f deploy/compose.yml \
  --project-name "$REVIEW_PROJECT" up --no-build --pull never --detach --wait
```

For Web development, additionally select `-f deploy/compose.dev.yml` and build its development image before starting:

```sh
docker compose --env-file compose.env \
  -f deploy/compose.native-review.yml -f deploy/compose.yml -f deploy/compose.dev.yml \
  --project-name "$REVIEW_PROJECT" build web
docker compose --env-file compose.env \
  -f deploy/compose.native-review.yml -f deploy/compose.yml -f deploy/compose.dev.yml \
  --project-name "$REVIEW_PROJECT" up --no-build --pull never --detach --wait
```

Use those exact files and project for stop/restart. Use the [Intel Web guide](../docs/wiki/code/intel-web.md) and [browser test guide](../docs/wiki/code/react-ui-testing.md) for isolated Next and Playwright. The backend-only recipe starts no browser frontend.

### Isolated deployment verification

When a real Docker run is explicitly authorized, the direct smoke owns a fresh review name, images and project:

```sh
python3 deploy/verify.py --name fresh-review --platform linux/amd64 --web --atlas --source-journey
```

`--dev` requires `--web` and a build. `--no-build --image-ids <file>` checks exact preloaded image identities. The helper retains its owned files and stops only its project; it never uses a retired launcher. The separate `python3 deploy/native_cutover_smoke.py --name <running-review>` exercises a synthetic API journey against an already started owned backend. Neither path proves installed native GUI behavior or real Rust-BERT inference. Repeat on a native ARM64 builder for separate Linux ARM64 evidence; do not silently emulate.

## Optional maintenance and inference

Workstation readiness is a bounded read. Seed, index rebuild, Natural training and candidate activation are explicit durable operations with separate permissions, confirmed context/configuration revisions and Jobs receipts. Natural activation requires the exact candidate hashes, expected prior model revision and explicit synthetic-baseline consent where applicable. It never installs Rust-BERT. [Classifier contract](../docs/wiki/code/classifier.md)

A live Atlas deployment requires separately staged, hash-verified architecture-matching Linux inference assets and configured provider accounts. [Atlas dispatch](atlas-dispatch.md) describes that boundary. Backend service executables such as `argos-api-host` and administrative support scripts remain available; Venture, the six terminal clients and legacy `argos` are [retired](../docs/archive/cli-retirement.md).

## Configured native maintenance inputs

The gateway service host reads optional `FLEETWORKS_MAINTENANCE_INPUTS_FILE` at startup. It names a bounded JSON array of at most three unique `ConfiguredMaintenanceInput` entries. Each entry pins `operation`, `dataset` and the current positive `configuration_revision`; supported operations are `seed_actors`, `train_classifier` and `reindex_actors`. Seed/train require the matching already staged, validated `DatasetRef`; reindex requires `dataset: null`. For example, a reindex-only manifest for the actual current revision 7 is:

```json
[{"operation":"reindex_actors","dataset":null,"configuration_revision":7}]
```

Use the actual observed revision, not the example value. Missing, stale, unavailable or unauthorized inputs leave the corresponding native trigger disabled; readiness never invents a dataset or completes an operation. The native UI has no upload or dataset picker. Operator tooling must stage/validate inputs separately. For a container gateway, explicitly mount the manifest and configure the environment path inside that container; the provisioning helper does not synthesize configured datasets. Reloading this startup configuration requires restarting the affected gateway using its exact existing deployment selection.
