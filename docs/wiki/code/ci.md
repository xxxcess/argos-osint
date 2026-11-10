# CI workflows

[Code route](index.md) · [Shared agent execution policy](agent-workflow.md) · [Wiki](../../index.md)

Workflows live in `.github/workflows/` (owned by Code Architect).

[![Fleetworks CI scopes](../../diagrams/ci-scopes.svg)](../../diagrams/ci-scopes.html)

## `ci.yml` — build and test matrix

The compatibility matrix covers the existing `argos` executable and embedded and server storage contracts. It runs on pushes to `main` and when a non-draft PR is opened, reopened or marked ready. Use the `ci:full` label for an explicit full run on a PR. A manual dispatch (`gh workflow run ci.yml --ref <branch>`) runs the matrix on a branch. The eight legs use Rust 1.95.0, run without debuginfo and have a 90-minute timeout. Fleetworks PR updates use the scoped workflow below.

| Platform | Runner | Legs |
|---|---|---|
| Linux x86_64 | `ubuntu-24.04` | embedded, server |
| Linux aarch64 | `ubuntu-24.04-arm` | embedded, server |
| macOS arm64 | `macos-15` | embedded, server |
| macOS Intel | `macos-15-intel` | embedded, server |

Every leg calls the same entry point you run locally, `scripts/local_matrix.sh`:

- **Embedded** (default features `embedded,nlp,classifier`): installs libclang on Linux, then `local_matrix.sh --storage embedded` = `cargo fmt --check` → clippy `-D warnings` → `cargo test --workspace --locked` → `test_surreal.py --mode embedded` (all `storage_*` tests on in-process RocksDB).
- **Server** uses `--no-default-features --features server,nlp,classifier` and resolves the pinned server from the release manifest. The harness verifies downloaded archives against the manifest's SHA-256 pins. CI caches the binary and passes its exact path to `local_matrix.sh --storage server --surreal-bin <bin>`. That leg runs clippy on core and the CLI, followed by server storage tests. [SurrealDB release manifest](../../../scripts/surrealdb-release.json)

Storage tests are selected by `ARGOS_SURREAL_MODE`; with no mode and no `ARGOS_SURREAL_BIN` they skip, so the plain `cargo test` in the embedded leg doesn't duplicate them. `test_surreal.py` sets `ARGOS_SURREAL_REQUIRED=1`, so a missing server fails instead of skipping.

Cache: `Swatinem/rust-cache` keyed per OS × storage, covering `target/agents` (where `scripts/agent_cargo.py` builds).

The native embedded leg is the [local compatibility gate](contributing.md#full-local-embedded-gate--github-ci-monitoring) for affected embedded or legacy runtime changes. Keep the full server, all-platform and mutation matrices in hosted CI. Use the [focused recipes](contributing.md#targeted-local-rust-builds-and-tests-fast-iteration) for an authorized isolated network fixture.

## `fleetworks.yml` — scoped branch checks

The [Fleetworks workflow](../../../.github/workflows/fleetworks.yml) runs on pushes to `main` and `feat/atlas-zero-shot`, PR updates and manual dispatch. It runs the full selection on the default branch once the scheduled workflow is present there. The nightly cron is `17 6 * * *`. The [shared planner](../../../scripts/fleetworks_verify.py) selects jobs from the event commit and changed paths. Markdown and documentation figures skip runtime scopes. Shared Rust contracts widen selection. Unknown paths, planner edits and unavailable comparisons select all scopes. The workflow cancels stale PR runs.

| Scope | Hosted evidence |
| --- | --- |
| Native clients | Linux x86-64, Apple silicon macOS and Intel macOS tests and executable smoke for Venture and its sibling clients |
| Server | Linux ARM64 service check, build, unit tests and network bootstrap and readiness smoke |
| Storage and broker | Affected disk storage and brief recovery contracts, plus RabbitMQ-backed Atlas dispatch when selected |
| Packaged Compose | Venture-owned image startup, authenticated readiness, relay and Atlas smoke on Linux ARM64 and AMD64 |
| Intel Rust-BERT | Intel macOS worker build, link and real checkpoint inference |
| Web | Locked install, static and unit checks, production build, relay lifecycle and production Chromium contracts. The development browser profile also runs for selected runtime changes. |

The `Fleetworks required` job runs after scope planning and reports the exact event revision. A selected job must succeed. An unselected job is expected to skip. Failed scope planning and missing, failed, cancelled or unexpectedly skipped selected jobs fail the aggregate. Storage and broker checks depend on the server job. The development browser check depends on the web job.

Full mode adds the pinned server storage suite and an AMD64 packaged development readiness check. The native, server and Compose jobs retain timed-phase JSONL and available Cargo timing HTML for seven days. Each record gives the phase, elapsed time, exit status, revision and machine. Use comparable completed runs for timing comparisons.

Preview the scopes selected for the checked-out revision or run the affected local checks.

```sh
base="$(git rev-parse HEAD^)"
python3 scripts/fleetworks_verify.py plan --base "$base" --head "$(git rev-parse HEAD)"
python3 scripts/fleetworks_verify.py local --base "$base"
```

Push selection compares the two commits directly. Add `--event pull_request` to compare against the merge base. Omitting `--base` selects all scopes. `--head` rejects a mismatched checkout. Local mode also includes working-tree paths and runs static checks plus selected native and web unit checks. It reports the hosted checks still needed.

`pr --base <sha>` and `full` select hosted scopes without starting a workflow. Actions can read those selections via `--github-output`. An explicit dispatch needs an authenticated `gh` and a remote ref at the checked-out commit.

```sh
python3 scripts/fleetworks_verify.py dispatch pr --ref <branch> --base <sha>
python3 scripts/fleetworks_verify.py dispatch full --ref <branch>
```

A Compose configuration check confirms the merged model resolves. The packaged smoke starts the actual containers. Report both by revision and distinguish source-built smoke evidence from published release-artifact verification.

## Efficient local verification before CI

Follow the [scope and freeze policy](agent-workflow.md#scope-and-finish-line) and [contributor commands](contributing.md). Use one shared-cache build owner and stable features. For Fleetworks, use the scoped local and hosted checks. For affected compatibility code, run the native embedded gate with parent storage-mode variables unset. Record local, hosted and skipped evidence separately.

## `datasets.yml` — LFS dataset integrity

Runs for dataset, `.gitattributes` or its own workflow changes on `main` and pull requests, plus manual dispatch. This is the only job that requests an LFS checkout of the full actor catalog. `git lfs fsck --objects --pointers HEAD` verifies committed pointers and LFS objects; a metadata-only check confirms `datasets/argos_actors.csv` is an LFS pointer in Git and its checkout has the declared full file size. It does not parse the catalog or seed a datastore.

Rust, mutation, web and policy checkouts explicitly leave LFS objects unmaterialized. Their tests use isolated synthetic catalogs; the large dataset is not embedded in Rust binaries. This avoids downloading the same catalog in every platform/storage leg. Checkout credentials are not persisted, and the Rust matrix has read-only contents permission.

## `atlas-bert.yml` — classifier worker checks

The workflow checks the standalone `tools/atlas-bert` workspace using Rust 1.95.0 and its committed lockfile. Direct worker pull requests run formatting and `rust-bert/doc-only` type checks on Linux.

The Fleetworks workflow calls the Intel runtime job for selected classifier changes and full dispatches. It verifies the event commit and uses the PyTorch 2.1.0 wheel for Intel macOS and Python 3.11. The job builds, links, tests, and exercises the real checkpoint on `x86_64-apple-darwin`. The native Venture and client executables have their own Intel lane.

For runtime verification, run `gh workflow run ci.yml --ref <branch> -f atlas_bert_intel_only=true`. This dispatch skips the standard matrix, links and tests the native Intel executable, prepares the pinned MNLI checkpoint with the same script as local setup, and compares its batched and repeated inference against one-item requests. Direct dispatch of `atlas-bert.yml` supports the same runtime checks with `-f intel_runtime=true`. Run the standard matrix separately for application compatibility builds and storage tests. See the [classifier guide](classifier.md) for local setup.

## `commit-policy.yml`

On PRs (including title/body edits) and pushes to `main`: `node scripts/release_check.mjs event` enforces Conventional Commits on the PR title/body or new `main` commits, `release_check.mjs versions` checks workspace/lockfile/manifest versions agree, and the `.opencode/tests` release/tool/planning tests run.

## `release-please.yml`

On pushes to `main` (or manual dispatch): Release Please opens/updates the release PR and tags from Conventional Commits (`release-please-config.json`, `.release-please-manifest.json`). Uses a GitHub App token or `RELEASE_PLEASE_TOKEN` so release PRs trigger CI; see [versioning.md](versioning.md).

## `fleetworks-candidate.yml` — release candidates

The [candidate workflow](../../../.github/workflows/fleetworks-candidate.yml) builds release binaries for Linux AMD64 and ARM64 and Intel and Apple silicon macOS. Start a run on a remote ref with `gh workflow run fleetworks-candidate.yml --ref <branch> -f head=<sha>` to pin the exact commit. The workflow uploads a CLI archive and its SHA-256 for each target and retains them for 14 days.

A separate native job downloads the exact CLI artifact by ID, verifies its archive and file hashes, checks the revision, version and executable architecture, and runs the installed Venture and client binaries.

Each Linux target also exports the runtime, web, SurrealDB and pinned RabbitMQ images. Verification downloads the exact image artifact and CLI artifact by ID, checks archive digests and native image IDs, then starts the downloaded Venture and images with `--no-build`. The smoke checks the image ID of every created service. The image archive is limited to 1 GiB compressed and 3 GiB uncompressed. The AMD64 job checks restart behavior for both Venture-managed host modes and an independently started external disk database with the pinned SurrealDB executable. These Actions artifacts verify release candidates. Public release distribution and registry manifest digest verification are separate. [Release policy](versioning.md#release-candidates)

## `intel-web.yml` — local web client checks

The web workflow runs directly for matching pushes and PR updates, and by manual dispatch. The Fleetworks aggregate also calls it with the exact revision and a development-browser selection. The direct and aggregate jobs can both run on a web PR while compatibility checks remain configured. Node comes from `.node-version` and pnpm is pinned to 12.15.0.

| Check | Enforced behavior |
| --- | --- |
| Frozen install and peer check | Reproducible lockfile and compatible dependency peers |
| `pnpm test` | Offline unit, component and Query/MSW contracts; Vitest configuration changes trigger this workflow |
| `pnpm typecheck` / `pnpm lint` | Type checking and recommended Hooks/compiler diagnostics with zero warnings |
| `pnpm build` | Next server build using the strict React Compiler pipeline; the upstream need not run at build time |
| `pnpm check:web-build` | Built Next configuration retains strict compiler options and RN Web/shared transpilation; compiled **application** chunks contain compiler memoization (framework runtime alone is insufficient) |
| `pnpm test:live` | Offline raw-relay, production runtime/restart and isolated development/HMR lifecycle contracts using the Atlas BERT worker fixture. Runs after the Next build and browser installation. |
| Offline Playwright | Chromium exercises the RN Web UI, compilation, hydration, connection and actions, and responsive layout against the production Next server. Direct workflow runs and selected aggregate runs also run the same fixtures against the development server. Production nonce CSP and same-origin API assertions apply to the production server. |
| Retained evidence | Export/dev test results and screenshots use separate directories under `test-results/`; HTML reports use `playwright-report/export` and `playwright-report/dev`. Uploaded with `always()` and seven-day retention. |

Local agents can isolate the production build/check pair with `ARGOS_WEB_TEST_DEV=1 pnpm build` followed by `ARGOS_WEB_TEST_DEV=1 pnpm check:web-build`. Both use the existing `.agent-scratch/next-web-tests` mapping and preserve launcher `.next`; omitting the setting keeps default build behavior. This pair verifies compiled output. Follow with `ARGOS_WEB_TEST_DEV=1 pnpm test:e2e` for the default production browser profile using that same isolated build; development remains `ARGOS_WEB_PROFILE=dev pnpm test:e2e`. Browser results and the full frontend gate remain separate evidence. Follow the [contributor isolation rules](contributing.md#build-and-test) and keep the user’s frontend running.

New PR web runs cancel stale runs for the same PR; main runs are not cancelled. The web job has a 25-minute limit for the build, live lifecycle tests and both browser profiles.

The evidence upload ignores absent files when an earlier step fails; it does not turn a missing build/test into a pass. Captures are review evidence, not automatic visual approval. No public deployment, live provider keys, secrets or native builds are involved. Path filters mean a docs-only PR does not run this job; configure required checks with that behavior in mind rather than assuming a workflow run for every PR.

Chromium browser fixtures disable LCD subpixel text and font hinting so the shared screenshot baselines use grayscale text on Linux and macOS. Keep the screenshot pixel threshold unchanged; inspect real layout or state differences rather than accepting them as rasterization noise.

The required-mode storage runner includes actor-catalog associations and article confidence/sorting contracts. A workspace test pass without a configured datastore can skip these checks; use the embedded/server harness for datastore evidence.

Worker lifecycle and ordered-queue fixtures own isolated test processes, private homes and Tokio runtimes. Repository initialization finishes before timing shutdown; the owning runtime stays alive until all pools and writes stop. The 500ms shutdown and 350ms Drop budgets remain enforced, and worker-thread panics fail the fixture.

## Supplemental process checks

For the Fleetworks packaged stack, use [Venture's Compose smoke](../../../deploy/verify.py). Its `--dev` option checks development readiness and shutdown. Browser reload is covered separately by the web fixtures.

For changes to the compatibility launcher, run `python3 scripts/test_web_stack.py --terminal-captures .agent-scratch/stack-review` after building `argos` and `argos-api-host` through the isolated Cargo wrapper and building the Next server. `--tui-only --tui-web-profile installed` narrows this to Venture back/value restoration, TUI return, same-parent storage reuse, shared API/Web TUI and terminal cleanup. These PTY/process checks are local supplemental evidence; the workflows above do not execute this script. Keep actual hosted results separate.

The composer-launch regression `python3 scripts/test_web_stack.py --composer-smoke`
runs the same PTY harness in its own temp `ARGOS_HOME`: `storage init --mode
embedded`, then `stack up --tui`, submit a recon prompt from the Home composer,
assert the investigation surface replaces the Home launcher, and then hold a
bounded 45 s window in which no `stack overflow`/`fatal runtime error` may
appear and the child must stay alive. It finishes with clean `ARGOS_EXIT=0` and
termios restored. It needs only `argos` built (no
`argos-api-host`) and is POSIX-only (skips elsewhere). Like the stack lifecycle
checks, it is local supplemental evidence that the workflows do not execute;
keep actual hosted results separate.

Live-transport changes also run `pnpm test:live` after the Next build. Its offline Node fixtures verify origin checks, raw relay framing, dropped upstream connections and the real Next runtime across a stateless restart. Browser fixtures intercept WebSockets as well as REST; request-count assertions cover quiet pages, push metrics, pending work and reconnect. The web workflow runs these contracts in its live relay/runtime lifecycle step after building Next and installing Chromium. Local executions and hosted results remain separate evidence.

## Scoped mutation diagnostics

[mutation.yml](../../../.github/workflows/mutation.yml) is a Linux embedded diagnostic gate triggered on pushes to `main`, separate from the platform/storage matrix. Its current trigger is defined in the workflow; there is no manual-dispatch trigger. It pins cargo-mutants 27.1.0 and uses [.cargo/mutants.toml](../../../.cargo/mutants.toml) to restrict mutation sites to NLP selection, provider attempt/request policy and Atlas classification, with the core library and property integration tests as the oracle. Supported default features are `embedded,nlp,classifier`; the workflow remains authoritative for any explicit mutation feature selection. No live providers or server storage are used.

The runner keeps the unmutated baseline check, fails on missed or timed-out mutations and retains outcome/log artifacts for seven days, including failure evidence. The 120-minute workflow limit bounds this expensive diagnostic; a timeout is incomplete evidence, not a passing baseline. Configuration and local smoke results do not establish a hosted mutation result. Review survivors before changing tests; equivalent mutants require a documented rationale rather than silently weakening the gate.

## Rust mutation execution restriction

Until further notice, agents must not run Rust mutation tests or mutation baselines locally, including `cargo mutants` and cargo-mutants runner invocations. Use the GitHub Actions `mutation.yml` workflow for execution and inspect its hosted results. Report local mutation execution as skipped. Run the ordinary targeted checks and any affected compatibility gate.
