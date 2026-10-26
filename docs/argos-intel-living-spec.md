# Argos Intel — Living Cross-Platform Implementation Specification

> **Plan v2.6 · 2026-10-25 · Status: Phase 1 locally verified**  
> Repository baseline: [`xxxcess/argos-osint` `main` @ `1c847e8`](https://github.com/xxxcess/argos-osint/commit/1c847e89357f49ee23942f48a11d1a6af7b6bc9a)  
> **One-file, incrementally maintained implementation contract.** The original baseline audit is retained below. Current implementation and evidence are tracked in section 9; later native-platform milestones remain Planned.

## 1. Delivery sequence

| Phase | Target | Deliverable | Status |
|---|---|---|---|
| **1** | **Web** | Local Next.js/React Native Web Intel application using the Rust API; public deployment deferred | Locally verified |
| **2** | **macOS Apple Silicon + Intel** | Separate Tauri 2 `.app` builds with architecture-matched Rust API sidecars | Planned |
| **3** | **Android** | Expo/React Native app accessing a reachable development Rust API | Planned |
| **4** | **iOS + Windows** | Expo native iOS app and Tauri 2 Windows installer with Windows Rust sidecar | Planned |

Complete and verify each phase before advancing, except small compatibility experiments. **The web app ships in Phase 1**; its static bundle is reused inside Tauri in Phases 2 and 4. Rust builds are required **on the API host** for web/mobile, while desktop also needs a **native Rust executable for each bundled target**. Tauri itself is not needed for the web, Android, or iOS clients in this architecture. **Every non-TUI application requires the Argos HTTP API to be ready before entering its working UI. The standalone TUI reads core/storage directly without an HTTP API. An API/Web stack may display the normal TUI while an owned in-process API shares its repository. API requests have no authentication gate in this initial phase; restrict access to trusted local development paths.**

## 2. Architecture and repository changes

Keep the existing `argos-osint-core`, `argos-osint-bin` CLI/TUI, and **SurrealDB 3.3.4** data model. Extend the repository instead of replacing them:

```text
crates/
  argos-osint-core/         # Existing; add UI-independent Intel read/actions service
  argos-osint-bin/          # Existing CLI/TUI; add `stack` launcher without regressions
  argos-osint-api/          # New; Axum routes, DTOs, readiness and bind restrictions
  argos-api-host/           # New; executable Rust HTTP service
apps/
  web-desktop/             # Phase 1 Next static web app
    src-tauri/              # Add in Phase 2 for macOS; extend in Phase 4 for Windows
  mobile/                  # Phase 3 Expo Android; Phase 4 iOS
packages/
  ui/                      # Shared React Native primitives, themes and tokens
  intel/                   # Bulletin, Focus Brief, Related, topology UI
  api-client/              # Typed HTTP hooks, Context and TanStack Query
package.json
pnpm-workspace.yaml
pnpm-lock.yaml
Cargo.toml                  # Add Rust API crates; account for Tauri workspace boundary
.github/workflows/        # Preserve existing Rust CI; add phase-specific checks
```

[Local Intel web architecture and launch lifecycle](diagrams/intel-web.html) ([SVG](diagrams/intel-web.svg)). Native clients reuse this boundary in later phases.

### Baseline repository audit (before Phase 1 implementation) — `main` @ `1c847e8` (2026-10-25)

**Observed, not proposed:** This is one commit after the previously audited `8984bca`. The Cargo workspace still contains only `argos-osint-core` and `argos-osint-bin`; the `argos stack` subcommands, Axum API host, Next/Expo workspaces and Tauri shell do **not** exist yet. Keep all platform milestones **Planned**. The changes relevant to this plan are:

- **New reusable binary-crate library:** `crates/argos-osint-bin/src/lib.rs` publicly exposes `cli` and `tui`; `src/main.rs` calls `argos_osint_bin::cli::dispatch()`. Extend `cli.rs` with the `Stack` command, and export new `stack` orchestration from `lib.rs` for integration tests. The current `main.rs` initializes the runtime before dispatch; `storage::runtime::init` records a handle/home without opening the repository. Avoid creating a second embedded owner when the future API host starts.
- **Current Intel TUI contracts:** `tui/app.rs`, `tui/ui.rs` and `tui/db_reads.rs` remain the behavior/reference paths. The new TUI frame cache and existing background Intel snapshots mean API DTOs should be **core-owned, bounded, revision-aware data**, not exported `App`, frame-cache or Ratatui types. Preserve the loading/refresh and selected-article semantics.
- **Database/query work:** `storage/topology.rs` now contains bounded, batched topology querying; `storage/event_log.rs` maintains durable event `seq` values with possible gaps; `storage/index_drain.rs` and `storage/outbox.rs` batch work. Reuse these paths for topology, job/events and indexing statuses rather than reimplementing loops or assuming contiguous event IDs. The new `core/src/http_client.rs` pools **outbound `reqwest` clients**—it is **not** the planned inbound Axum API server.
- **New regression evidence sources:** `crates/argos-osint-bin/tests/perf_tui_rebuild.rs` and `crates/argos-osint-core/tests/storage_perf.rs` exercise frame-cache and topology performance budgets. Keep these in the existing Rust gate; add API-specific pagination and readiness tests without asserting that frontend work improves those unrelated budgets.
- **Canonical documentation moved:** begin at `AGENTS.md` → `docs/index.md` → `docs/wiki/code/` and `docs/wiki/concepts/`. Old top-level files such as `docs/usage.md`, `docs/conventions.md`, `docs/opencode-v2.md` and `docs/tui-design-spec.md` are no longer canonical. The new repo wiki replaces Graphify as the implementation-navigation source; do not rely on removed `graphify-out` files or the removed `.opencode/skills/graphify/`.

**Planning integration:** Keep this cross-platform spec as one maintained Markdown contract under a stable `docs/` path when committed (suggested `docs/argos-intel-living-spec.md`, linked from `docs/index.md`). During work, use `.planning/2026-10-25-argos-intel/` with the new `task_plan.md`, `findings.md`, and `progress.md` templates for *local/ignored* execution state. Per `docs/wiki/code/planning-artifacts.md`, curate durable decisions back into this spec; do not commit temporary progress files merely to track status. When changes affect behavior/contracts, refresh the relevant canonical wiki page and HTML/SVG diagram in the same PR, with scoped Scribe/wiki checks.

### Shared implementation rules

- **Rust is the source of truth.** Move Intel read models currently coupled to `crates/argos-osint-bin/src/tui/db_reads.rs` into reusable core services; do not expose TUI `App`/frame cache through HTTP. Keep existing asynchronous snapshot/loading behavior. Reuse `storage/{repo,runtime,atlas,intel_brief_view,brief_stages,home,server}.rs`; keep the CLI/TUI behavior unchanged. Use indexed/cursor pagination instead of scanning all news cycles per search, and preserve query bounds/round-trip budgets for the topology path.
- **Version the API** under `/api/v1`: health/capabilities; day/category and article lists; article detail and Related; brief ensure/reload/stages; jobs/status/retry; topology. Validate IDs, input boundaries, idempotent retries, and stable result revisions. The static Next.js export **does not host this API**.
- **Mandatory API startup order:** for web/macOS/Android/iOS/Windows, prepare or adopt the Argos API **before** starting the frontend runner or opening its functional UI. API readiness requires successful unauthenticated health, working storage access and initialized required services—not merely an open TCP port. For web/mobile clients launched outside the CLI, display a connection/retry screen until the API becomes available; never present a misleading working state. **Building a release artifact does not require an already running API; running that artifact does.**
- **TUI independence:** the Ratatui TUI is the behavioral/visual reference and continues to use Rust core/storage directly. `argos` or `argos stack up --tui` neither creates an Argos HTTP listener nor waits for one. The optional SurrealDB server mode is a separate storage-concurrency concern.
- **Preserve storage ownership.** Embedded SurrealDB must have one owning process. Concurrent direct CLI and API processes need the repository's supported server-mode semantics. The Argos HTTP API host and the optional SurrealDB server are different processes.
- **Shared UI:** use React Native `View`, `Text`, `Pressable`, `FlatList`, with React Native Web for browser/Tauri and Expo for mobile. Keep Tauri/native functionality behind platform adapters (`.web.tsx` / `.native.tsx` when needed).
- **State:** React Context for theme, selected backend, preferences and connectivity; TanStack Query v5 for fetching, caching, search, pagination, mutations and refetch. Namespace cache by API instance ID and clear/revalidate on backend switch. Use component state for local panel selection. **No Convex.** Polling is an acceptable first step; add server-sent events only after it is needed and tested. If SSE uses `argos_event.seq`, treat it as increasing but **not gap-free**.
- **Intel TUI parity:** six category tabs; day/search/featured article; Confidence, Actors, Claims, Inferences and Links; full body, Related leads (not corroboration); distinct publication/indexing states; topology seed navigation. Images are a React Native client feature: category feed thumbnails sit left of card text, while the core terminal TUI is text-only. Operational Jobs/Logs stay in Rust. Wide Focus Brief columns ~**24% / 48% / 28%** (Extracted/Reader/Context); narrow layout uses tabs. Do not restore retired article Recon reports.
- **Initial API access (no auth gate):** do not implement API login, API keys, bearer tokens, session checks, or authentication-based readiness. `GET /api/v1/health/ready` and application endpoints are unauthenticated. **Bind the Rust API to `127.0.0.1` by default**; do not make it publicly reachable or publish a web/mobile deployment against an unprotected endpoint. Keep strict Host/Origin/CORS allowlists, safe input validation and a narrow CSP; these are protective controls, **not authentication**. Never expose SurrealDB root credentials or provider secrets to JS. Mobile development must use a scoped bridge/tunnel to a loopback-bound host rather than opening an unauthenticated LAN listener. Public access and production mobile networking require a separate security decision and are deferred.

## 3. Unified Argos stack launcher (cross-phase)

**Goal:** the existing `argos` binary becomes the single entry point for starting development stacks and already-built applications. **`argos` with no arguments shows an ASCII welcome and TUI/API Server/Web service selector**; preserve direct TUI access with `argos stack up --tui` and all existing `argos recon`, `storage`, `models`, `atlas`, etc. commands. `argos stack` implements the Phase 1 TUI/server/web targets; native target flags remain deferred until their phases are verified. **A running Argos API is a hard prerequisite for every launched application target except the TUI.**

### Proposed CLI contract

Use Clap's **mutually exclusive platform flags**, so one `up` invocation selects one stack. `--server` means the **Argos HTTP API**, not the existing SurrealDB server (`argos storage start`).

```sh
argos                                        # Current behavior: TUI
argos stack up --tui                         # Explicit TUI; never starts API
argos stack up --server --api-port 8080      # Rust API host only; no UI
argos stack up --web --port 3000             # Ensure API ready -> Next.js on localhost:3000

argos stack up --macos --arch arm64          # Tauri dev stack on Apple Silicon
argos stack up --macos --arch x86_64         # Tauri dev stack on an Intel Mac
argos stack up --android --device auto       # Ensure reachable API -> Expo/Android
argos stack up --ios --device auto           # Ensure reachable API -> Expo/iOS (macOS host)
argos stack up --windows                     # Tauri dev stack (Windows host)

argos stack status                           # Managed processes and health
argos stack down --web                       # Stop only stack-owned processes
argos stack doctor --target web              # Check prerequisites, no installation
argos stack doctor --target macos --arch arm64
argos stack build --web                      # Explicit production artifact creation
argos stack build --macos --arch arm64       # Phase 2+ architecture-specific artifact
```

- Exactly **one** of `--tui | --server | --web | --macos | --android | --ios | --windows` is required for `stack up`. `--port` applies **only to web** (default `3000`); `--api-port` applies to an **owned local API** (default: allocate a free port). Keep web/API ports distinct; fail clearly on a port collision. Bind local development services to `127.0.0.1`; do not offer an unauthenticated LAN/public listener in the initial implementation. The web URL prints as `http://localhost:3000`; do not serve unprotected intelligence on `0.0.0.0`.
- `--arch auto|arm64|x86_64` applies to macOS (and an appropriate Windows target in Phase 4); `up` must verify a runnable host/architecture instead of pretending cross-compilation means it can execute. `build` may cross-compile **only when the full native toolchain works**, and must stage a matching Tauri sidecar.
- `--device auto|<device-id>` applies to Android/iOS; resolve through `adb`/Expo or Xcode/Simulator, not ad-hoc shell strings. `--api-url <http://127.0.0.1:PORT>` (or a trusted, loopback-forwarded equivalent) selects an existing development API. An Android device can use `adb reverse tcp:PORT tcp:PORT` so its `127.0.0.1:PORT` reaches the host; iOS Simulator can use the Mac host loopback. Do not assume that an arbitrary phone's `localhost` is the host. Physical iOS/non-USB remote access is deferred until a scoped secure access strategy exists.
- `--profile dev|installed` (default `dev` in a source checkout) distinguishes development runners from existing built apps. `up` **does not install toolchains or build release packages**; a native dev runner may perform normal incremental compilation. `build` is the explicit release-artifact path; `installed` must fail clearly if no executable exists.
- `stack down`/`status` track actual process identity, managed instance ID, bind address and health, not just a possibly recycled PID. Signal handlers clean up only child-owned processes, release leases and preserve storage; never run `argos storage stop --force` implicitly.

### Required launch sequence (all non-TUI targets)

[Local Intel web architecture and launch lifecycle](diagrams/intel-web.html) ([SVG](diagrams/intel-web.svg)). Native clients reuse this boundary in later phases.

1. **Preflight:** resolve target/toolchain, API configuration, loopback/bridge reachability and storage ownership. Do not start an API for `--tui`.
2. **Start or adopt API:** for any other target, reuse a compatible healthy Argos API instance or start **one** host under the right lifecycle owner. Keep `argos storage start` (SurrealDB server) separate from the **Argos HTTP API** (`--server`).
3. **Await readiness:** perform bounded retries against **unauthenticated** `GET /api/v1/health/ready`, including storage and mandatory service initialization; treat API configuration/storage errors as failures, not readiness. Only then start the frontend process, open browser/window or deploy/run the device app.
4. **Monitor:** if the API later disconnects, TanStack Query and the connection UI enter `offline/reconnecting` state and block writes until service recovery. Never hide startup failures behind empty Intel data.
5. **Stop safely:** on `stack down`/signal, close only owned frontend and API children; an adopted API remains running. Match process identity/instance tokens before cleanup, respect storage leases and never force-stop unrelated database processes.

**Ownership and distribution:** in source/dev mode, the CLI supervisor owns the local API it starts. In packaged desktop mode, **Tauri bootstraps/adopts its own bundled API sidecar** and gates the usable window until ready—never launch a duplicate sidecar from the CLI and Tauri simultaneously. For Android/iOS, the Rust API runs on a **separate machine/host**; `stack up` may start a reachable local development host when a local loopback-forwarding bridge is available; otherwise require a preconfigured, safely reachable API endpoint and verify it before launching Expo. A phone's `localhost` is not the Rust host. For a later public web deployment, design and approve API protection first; until then, the static web build is served locally and shows a connect/retry state if the loopback API is unavailable. For any device/app opened directly outside `argos stack`, client connection gating still applies.

**Build versus run:** `argos stack build --target` only compiles/packages assets and checks toolchains. **Readiness is a runtime requirement** for `stack up`, Tauri app startup and deployed web/mobile operation—not a dependency of offline compilation.

### Stack orchestration by target

| Stack flag | Phase | Runner / owned components | Host and tools |
|---|---|---|---|
| `--tui` | 1 | Existing in-process Ratatui TUI; **no API startup/check** | Existing Rust build; any supported CLI host |
| `--server` | 1 | Start/adopt `argos-api-host`, wait for readiness; **no UI** | Native Rust build; no Next.js/Tauri |
| `--web` | 1 | **API ready first**, then `pnpm --dir apps/web-desktop exec next dev -H 127.0.0.1 -p 3000` | Native Rust, Node, pnpm; web port defaults 3000 |
| `--macos` | 2 | **API ready first**, then Tauri dev; packaged app bootstraps/adopts local Rust sidecar | **macOS**, Xcode CLT, Rust target matching selected arch |
| `--android` | 3 | **Reachable development API ready first**, then Expo `run:android`/Metro | Android SDK/JDK/adb/Expo; Rust build on API **host**, not device |
| `--ios` | 4 | **Reachable development API ready first**, then Expo `run:ios`/Metro | **macOS** + Xcode/Simulator/Expo; Rust build on API **host** |
| `--windows` | 4 | **API ready first**, then Tauri dev; packaged app owns/adopts Windows sidecar | **Windows**, MSVC/Windows SDK/WebView2/Rust |

In `--profile installed`, web serves the static `out/` bundle instead of launching Next's dev server; desktop launches an existing signed/built app (which owns its sidecar). `next start` **must not** be used with `output: 'export'`. Mobile `installed` means opening a previously installed app when the host/device mechanism is supported; otherwise report that direct launching is unavailable.

### Repository changes and safety

- In `crates/argos-osint-bin/src/cli.rs`, add `Command::Stack { command: StackCommand }` and route no-argument launches through the `welcome` service selector to the existing `stack up` targets. Export the new implementation from the **existing** `crates/argos-osint-bin/src/lib.rs` for integration tests; leave `main.rs` as the thin runtime/dispatch entry. Put platform orchestration in a **separate** `src/stack/` module (CLI parsing/validation, capability detection, process supervisor and platform runners); don't expand the existing CLI's unrelated command implementations.
- Reuse the API-host crate for `--server`; `--web`, `--macos`, `--android`, `--ios` and `--windows` must all start/adopt or verify **one** appropriate API instance and wait for unauthenticated **storage-backed readiness** before launching the frontend. `--tui` skips all API orchestration. Prefer parameterized `tokio::process::Command` over shell-interpolated commands, set an explicit workspace `cwd`, propagate error codes/logs and avoid leaking SurrealDB credentials/provider secrets into argv, logs and environment dumps.
- The current `src/main.rs` calls `storage::runtime::init` before dispatch, which itself does not open the repository; ensure launcher-only orchestration does not subsequently open/lock embedded storage in a second process. Choose **one datastore owner**. When a TUI and API need concurrent direct database access, either use the existing **SurrealDB server mode** and its lease checks or later route TUI reads through a shared host; do not silently switch storage modes, bypass locks or start two embedded owners.
- `doctor` checks local Rust/Node/pnpm/Expo/Tauri/SDK/target prerequisites and API reachability when an endpoint is configured; prints precise installation guidance from the phase section but never installs/upgrades tools without a separate user action. Validate unsupported OS/target, missing native binaries, nonexistent working tree and inaccessible or unsafe API URL **before** spawning any child.
- Test Clap parsing/conflicting flags, platform capability checks, mocked child lifecycle, Ctrl-C propagation, restart/idempotency, API readiness (including storage readiness and delayed startup), API-down refusal, reconnect UI, port collision, loopback bind restrictions, Origin/CORS checks, storage locks, no API creation for TUI, adopted-server retention, and **no child orphaning**. Existing `argos` and `argos storage` regression tests remain mandatory.

**Delivery mapping:** implement `--tui`, `--server` and `--web` in Phase 1; activate `--macos` in Phase 2, `--android` in Phase 3, and `--ios`/`--windows` in Phase 4. Earlier CLI flags should print **“target not yet implemented”** until their phase is verified; never claim launch support merely because flags parse.

## 4. Shared toolchain and update policy

- **Rust:** preserve repository-pinned **1.95.0** toolchain and existing `Cargo.lock`; use Cargo, `rustfmt`, `clippy` and an OS-appropriate C/C++ compiler/linker for SurrealDB/RocksDB dependencies. Do not upgrade SurrealDB 3.3.4 without a separate migration/test decision.
- **JavaScript:** use **Node.js 24 LTS** and **pnpm 12** as the plan's starting toolchain; record the **exact tested pnpm version** in root `packageManager`, lock all dependencies with `pnpm-lock.yaml`, and use `pnpm install --frozen-lockfile` in CI. Select **compatible, explicitly pinned** Next, Expo SDK, React, React Native, React Native Web, Tauri 2 and React Compiler versions by running a cross-platform compatibility spike before locking them. Do not assume latest versions are compatible.
- **Installation:** install Rust using [rustup](https://rustup.rs/), Node LTS via a version manager or official installer, and pnpm via its [official installation instructions](https://pnpm.io/installation/). Add `engines` and tool-version files as appropriate. Use `expo install` for native Expo dependencies.
- **Upgrades:** update tool versions/dependencies in a scoped PR, regenerate Cargo/JS lockfiles, run all verified phases' regressions, and record versions, breaking changes, commands and artifacts in the evidence tracker. Keep React Compiler opt-in until Next/Expo compatibility passes. Avoid one-off global tool versions that differ from CI.
- **Rust regression gate:** follow [AGENTS.md](https://github.com/xxxcess/argos-osint/blob/main/AGENTS.md), [contributor workflow](https://github.com/xxxcess/argos-osint/blob/main/docs/wiki/code/contributing.md) and the existing `.github/workflows/ci.yml`. For agent builds use `python3 scripts/agent_cargo.py … --locked` (not the user's `target/debug`); locally run only `scripts/local_matrix.sh --storage embedded` in the combined integration tree, while the **server-storage leg stays in CI**. Include new `perf_tui_rebuild` and `storage_perf` regressions, CLI/TUI/storage contracts and API tests; follow the documented run-count cap.

## 5. Phase 1 — Web

**Output:** static `apps/web-desktop/out/` served locally (default `http://localhost:3000`); browser connects to a separately running loopback-bound `argos-api-host`. **No Tauri build.** In local `stack up --web`, the API becomes ready **before Next.js starts**; for a directly opened local app, show reconnect UI if its API is down. **Public hosting is deferred while API auth is absent.**

**Required tools:** Rust 1.95/Cargo, C/C++ compiler/linker, Tokio/Axum, SurrealDB 3.3.4, Argos stack launcher/Clap process supervisor; Node 24 LTS, pnpm 12, TypeScript, React, Next.js (`output: 'export'`), React Native Web, TanStack Query v5; local static hosting; typecheck/lint and browser E2E tooling (e.g. Playwright).

**Install/build commands** *(app/API package paths become runnable after their milestones are implemented)*:

```sh
rustup toolchain install 1.95.0 --component rustfmt clippy
rustc +1.95.0 -V; node -v; pnpm -v
cargo +1.95.0 build --locked -p argos-api-host
pnpm install --frozen-lockfile
pnpm --dir apps/web-desktop build        # Next static export -> out/
# `argos stack up --web --port 3000` first verifies the loopback API, then runs Next locally.
```

**Constraints:** Next static export cannot execute runtime Next API routes, server actions or request-time SSR. Rust is compiled for **the API host OS/CPU**, not WebAssembly. Build the first stable API, then the Bulletin and Focus Brief; include responsive navigation, API readiness and connection/error states.

**Launcher acceptance:** `argos` and `argos stack up --tui` run directly without starting/waiting for the API. `--server` reaches unauthenticated storage-backed readiness; `--web --port 3000` **waits for the API before starting Next** at `http://localhost:3000`; API failure blocks web launch, and `status`/`down` avoid orphans.

**Acceptance:** local browser can browse/search articles, open a Focus Brief to idempotently admit/attach its core extraction chain, read full briefs, show Related, navigate topology, and exercise article mutations; operational Jobs/Logs stay in Rust. Running API/Web services remain reachable on crowded bounded Jobs pages, with scoped traffic events. Rust storage contracts and browser E2E pass; localhost-only API binding, CORS, Origin, CSP and sensitive-secret separation reviewed; no remote/public API acceptance.

## 6. Phase 2 — macOS Apple Silicon and Intel

**Output:** Tauri 2/WebKit `.app` for each architecture using the verified Phase 1 static UI and target-matched `argos-api-host` sidecar. Start with **separate bundles**; universal builds are a later optional enhancement.

**Required tools:** all Phase 1 build dependencies, Xcode Command Line Tools and macOS SDK, Tauri 2 Rust crate/JS CLI, `aarch64-apple-darwin` and `x86_64-apple-darwin` Rust targets; signing/notarization tools for public distribution. Full Xcode is optional for macOS-only builds. Native C/C++ dependencies must actually link for both targets.

```sh
xcode-select --install                 # if Command Line Tools are missing
rustup target add --toolchain 1.95.0 aarch64-apple-darwin x86_64-apple-darwin
xcode-select -p; xcrun --show-sdk-path
pnpm install --frozen-lockfile
pnpm --dir apps/web-desktop build

# Apple Silicon
cargo +1.95.0 build --locked --release -p argos-api-host --target aarch64-apple-darwin
# Stage target binary as src-tauri/binaries/argos-api-host-aarch64-apple-darwin
pnpm --dir apps/web-desktop exec tauri build --target aarch64-apple-darwin --bundles app

# Intel
cargo +1.95.0 build --locked --release -p argos-api-host --target x86_64-apple-darwin
# Stage target binary as src-tauri/binaries/argos-api-host-x86_64-apple-darwin
pnpm --dir apps/web-desktop exec tauri build --target x86_64-apple-darwin --bundles app
```

**Constraints:** Tauri `externalBin` filenames require the target triple; renaming an ARM binary does not create an Intel binary. Check both binary architectures, local API startup/adoption, port collisions, shutdown and database process ownership. **P2-03 requires a smoke test on actual Intel Mac hardware**, not merely Rosetta. Set a deployment target supported by native dependencies. Retest Phase 1 after shared changes.

**Launcher acceptance:** `argos stack up --macos --arch arm64|x86_64` checks host/architecture, starts/adopts the matching local API, **waits for storage-backed readiness before Tauri UI startup**; wrong-host combinations fail before compilation.

**Acceptance:** arm64/Intel bundles install, launch, connect to their loopback API, load Intel data, handle keyboard/window lifecycle and exit cleanly; architecture-specific CI/artifacts and distribution signing plan are recorded.

## 7. Phase 3 — Android

**Output:** installable Expo/React Native Android app using `packages/{ui,intel,api-client}` and the separately running development Rust API. **No Android Rust cross-compile or Tauri build is required** with the client/server architecture.

**Required tools:** Node 24 LTS, pinned pnpm/Expo SDK/React Native versions, Android Studio, Android SDK/platform/build-tools, `adb`, Android emulator or physical device, Expo-compatible JDK and Gradle wrapper; optional EAS CLI for cloud distribution. The API host separately needs the appropriate Rust/C++ build environment.

```sh
# First install Android Studio/SDK and compatible JDK; configure ANDROID_HOME/PATH.
node -v; pnpm -v; java -version; adb version
pnpm install --frozen-lockfile
pnpm --dir apps/mobile exec expo install --fix
pnpm --dir apps/mobile exec expo start
pnpm --dir apps/mobile exec expo run:android
pnpm --dir apps/mobile exec expo-doctor   # add expo-doctor to dev dependencies
# For APK distribution, configure eas.json and an explicit preview profile.
```

**Constraints:** emulator `10.0.2.2` can connect to a debug host only when explicitly configured; physical devices require a scoped loopback bridge (e.g. Android `adb reverse`); public/release distribution using a remotely reachable unauthenticated API is deferred. Implement narrow-screen Extracted/Reader/Context tabs, connection configuration, offline/error and reconnect behavior. Test on emulator **and** physical device, and rerun shared web tests after upgrades.

**Launcher acceptance:** `argos stack up --android` validates SDK/device readiness, **starts or verifies a reachable loopback-bridged API before launching Expo**, and displays reconnect state for independently opened apps; it does not mistakenly run the Rust API on the phone.

**Acceptance:** native installable development APK; brief operations work over a loopback/ADB bridge; app resumes/reconnects reliably; Android CI and device evidence linked.

## 8. Phase 4 — iOS and Windows

### iOS (P4-01)

**Output:** native Expo iOS build using the Phase 3 mobile workspace and a development Rust API. No Tauri or on-device Rust is planned.

**Required tools:** Phase 3 shared Expo/Node/pnpm stack, **full Xcode** on macOS, iOS SDK, Simulator, provisioning/signing for devices, CocoaPods **if required by native dependencies**, and separately hosted Rust API.

```sh
xcodebuild -version
pnpm install --frozen-lockfile
pnpm --dir apps/mobile exec expo install --fix
pnpm --dir apps/mobile exec expo run:ios
# Configure Apple signing and EAS/Xcode release workflow for distribution.
```

**Launcher acceptance:** `argos stack up --ios` requires a macOS/Xcode host, **verifies unauthenticated API readiness over a supported development bridge before launching Expo**, and handles simulator or selected device; non-macOS requests fail with prerequisite guidance.

**Acceptance:** simulator tests, lifecycle/reconnect and signing/build readiness; physical-device network testing is blocked until a protected transport is approved, and unchanged Android behavior. Rust Apple iOS targets are unnecessary unless a separately approved future change embeds Rust in the app.

### Windows (P4-02 / P4-03)

**Output:** Tauri 2/WebView2 installer with a Windows-native `argos-api-host.exe` sidecar and the Phase 1 static UI.

**Required tools:** Rust 1.95/Cargo, MSVC toolchain (`Desktop development with C++` via Visual Studio Build Tools), Windows SDK, WebView2 Runtime, target `x86_64-pc-windows-msvc`, Node 24 LTS, pnpm 12, Tauri 2, native SurrealDB/RocksDB dependencies, installer/signing tools.

**Known portability work in existing code:** `storage/home.rs` uses `/dev/urandom`; `storage/server.rs` uses Unix `ps`/`kill`; `storage/install.rs` lacks a Windows SurrealDB download/checksum path. Fix and test those without weakening storage or process safety.

```powershell
# Install MSVC Build Tools, Windows SDK, WebView2, Node/pnpm and rustup first.
rustup toolchain install 1.95.0 --component rustfmt clippy
rustup target add --toolchain 1.95.0 x86_64-pc-windows-msvc
cargo +1.95.0 build --locked --release -p argos-api-host --target x86_64-pc-windows-msvc
# Stage Windows executable as src-tauri/binaries/argos-api-host-x86_64-pc-windows-msvc.exe
pnpm install --frozen-lockfile
pnpm --dir apps/web-desktop build
pnpm --dir apps/web-desktop exec tauri build --target x86_64-pc-windows-msvc
```

**Launcher acceptance:** `argos stack up --windows` **starts/adopts and validates the Windows-native API before Tauri UI startup**, and is rejected cleanly on unsupported hosts.

**Acceptance:** Windows-native core/API/storage and CLI tests pass; sidecar starts securely, installer installs/launches/uninstalls, Intel UI and lifecycle smoke tests pass on a **native Windows runner**. Retest earlier web/macOS releases for shared upgrades.

## 9. Incremental milestone tracker

**Status values:** `Planned` → `In progress` → `Verified`; use `Blocked` with reason. Never mark `Verified` without an actual command/test/CI run and reference to its output or artifact. Update this table in every implementation PR.

| ID | Scope | Status | PR / evidence / blocker |
|---|---|---|---|
| P1-01 | JS monorepo, pinned versions, shared RN Web component, Next static export | Verified | Node 24.22.1 / pnpm 12.15.0; frozen install, peer check, typecheck/lint/static export and browser fixture E2E pass; [web contract](wiki/code/intel-web.md) |
| P1-02 | Rust Intel service, unauthenticated API host, SurrealDB ownership and storage-backed readiness | Verified | `scripts/local_matrix.sh --storage embedded` green locally; API contract tests in that suite; [web contract](wiki/code/intel-web.md) |
| P1-03 | Web Bulletin/Focus Brief/Related/topology; Context + Query | Verified | Browser fixture E2E 11/11; opening/Related/revisit auto-admission and error/retry covered; Jobs/Logs/job-stage polling remains excluded from RN |
| P1-04 | Local web hosting, loopback/Origin/CORS hardening, browser E2E and existing Rust CI | Verified | Browser CSP/gating E2E; local embedded Rust gate; `intel-web.yml` present. Hosted Actions not run (unpublished) |
| P1-05 | Clap launcher: TUI independent; server-before-web startup, health, lifecycle, doctor/tests | Verified | `python3 scripts/test_web_stack.py` PASS after `argos`/`argos-api-host` build; explicit `storage init` required before owned API start |
| P2-01 | Tauri shell; start/adopt sidecar, await readiness before usable UI | Planned | — |
| P2-02 | macOS arm64 bundle and native smoke test | Planned | — |
| P2-03 | macOS x86_64 bundle and real Intel Mac smoke test | Planned | — |
| P2-04 | macOS UX/lifecycle, both-arch CI, signing readiness | Planned | — |
| P2-05 | CLI `--macos --arch` launch/build, native host/sidecar validation | Planned | — |
| P3-01 | Expo Android setup, shared UI, native debug build | Planned | — |
| P3-02 | Scoped Android loopback bridge, mobile layout, reconnect and device tests | Planned | — |
| P3-03 | Installable APK and CI/release readiness | Planned | — |
| P3-04 | CLI `--android`: unauthenticated API readiness through development bridge before Expo/device launch | Planned | — |
| P4-01 | iOS Simulator, API bridge, signing/build readiness; physical network access deferred | Planned | — |
| P4-02 | Windows Rust core/storage/API host portability | Planned | — |
| P4-03 | Tauri Windows installer, Windows Intel UI and native CI | Planned | — |
| P4-04 | CLI `--ios`/`--windows`: API-first runners, prerequisites and tests | Planned | — |

### Evidence entry template

```md
### YYYY-MM-DD — P#-##: short result
- Status: Planned → In progress → Verified (as applicable)
- PR/commit: <URL>
- Target and host: <platform, OS version, CPU architecture>
- Toolchain: <Rust, Node/pnpm, framework/native SDK versions>
- Command/test: <exact command and outcome>
- Artifact/CI: <link and optionally checksum>
- Blockers: <none or details>
- Next: <milestone>
```

## 10. Mandatory maintenance rules

1. Before a milestone, re-audit the relevant current `main` code and identify its test/acceptance conditions. Preserve existing CLI/TUI, Intel semantics and SurrealDB storage contracts.
2. During work, move its tracker row to **In progress**; on a genuine blocker, mark **Blocked** and record resolution criteria.
3. In the implementing PR, update the affected **phase section**, build requirements/versions, tracker row, canonical wiki and affected diagrams, and a dated **Change log** entry below. Provide PR, host/CPU, exact test command, result and CI/artifact link.
4. Mark **Verified** only after target-specific acceptance passes. CI configuration, cross-compilation or screenshots alone are not proof that a native app actually runs.
5. Record new/reversed architectural choices in the **Decision log** below. Do not erase superseded decisions or previous evidence.
6. For dependencies/toolchains, pin changes and rerun all previously verified targets, not just the phase being implemented.
7. Follow `docs/wiki/code/planning-artifacts.md`: local `.planning/` task checklists/progress are execution state; update durable spec decisions and relevant wiki references in the implementation PR. Use `docs/wiki/maintenance.md` scoped refresh, and never reference removed Graphify outputs as live repo knowledge.

## 11. Architecture decision log

| ID | Date | Decision | State |
|---|---|---|---|
| ADR-01 | 2026-10-25 | Initial Phase 1 macOS arm64/Intel + Android | Superseded by ADR-07 |
| ADR-02 | 2026-10-25 | Tauri 2 + Next static/RN Web for desktop; Expo for mobile | Active |
| ADR-03 | 2026-10-25 | Rust core + SurrealDB remain authoritative; authenticated Argos API | Superseded by ADR-14 |
| ADR-04 | 2026-10-25 | React Context + TanStack Query v5; no Convex | Active |
| ADR-05 | 2026-10-25 | Separate macOS arm64 and Intel bundles first | Active |
| ADR-06 | 2026-10-25 | Android is an API client; no on-device Rust; loopback bridge for local development | Active |
| ADR-07 | 2026-10-25 | **Phases: Web → macOS ARM/Intel → Android → iOS + Windows** | Active |
| ADR-08 | 2026-10-25 | Web/mobile use server-hosted Rust; desktop packages target-native Rust sidecars | Active |
| ADR-09 | 2026-10-25 | One Next static export feeds web and Tauri bundles | Active |
| ADR-10 | 2026-10-25 | Public web requires HTTPS/auth/CORS/CSP security review | Deferred; public hosting not in initial scope |
| ADR-11 | 2026-10-25 | One Clap `argos stack` launcher; exclusive target flags and explicit build/doctor/lifecycle | Active |
| ADR-12 | 2026-10-25 | CLI orchestration never introduces a second embedded SurrealDB owner; mobile connects to a bridged API host | Active |
| ADR-13 | 2026-10-25 | **API-first launch for all app targets; TUI alone is standalone and server-independent** | Active |
| ADR-14 | 2026-10-25 | **Temporarily omit HTTP API authentication; enforce local-only binding and development bridges; defer public deployment** | Active |
| ADR-15 | 2026-10-25 | Rebase implementation against `1c847e8`: canonical repo wiki, exported CLI/TUI library, async snapshot and bounded topology paths, new performance tests | Active |
| ADR-16 | 2026-10-25 | Maintain one committed living `docs/` spec; use repo-native `.planning/` files for local task execution and evidence | Active |

## 12. Change log

- **2026-10-26 — launcher UX update:** no-argument `argos` now shows the Argos OSINT ASCII welcome and a TUI/API Server/Web selector with storage mode, service ports, web profile, API adoption and a final TUI/logs/cancel terminal-display question. Web launches open the ready URL through the OS default browser; `--no-open` suppresses this. Owned TUI/API stacks share one in-process repository, preserving the embedded single-owner rule. Existing storage modes are preserved; new-home initialization runs in a child process before the selected datastore owner starts. Explicit `stack up` commands still launch directly. This user-requested behavior supersedes the original no-argument TUI default.

- **2026-10-25 — v1.0:** introduced a living milestone tracker initially scoped to macOS ARM/Intel and Android. Superseded scope; no implementation verified.
- **2026-10-25 — v2.0:** changed delivery order to Web → macOS ARM/Intel → Android → iOS/Windows; reorganized phase build tools and milestones. Documentation only.
- **2026-10-25 — v2.1:** consolidated the master tracker, shared contract, all platform build instructions, decision log and changelog into **this single Markdown file**. Documentation only; all milestones remain Planned.
- **2026-10-25 — v2.2:** added one cross-platform `argos stack` CLI contract for TUI/API/web/macOS/Android/iOS/Windows with `up`, `build`, `doctor`, `status`, `down`, prerequisites, security and milestones. **Specification only; commands do not yet exist.**
- **2026-10-25 — v2.3:** mandated API start/adoption and authenticated, storage-backed readiness **before running every non-TUI frontend**, with TUI standalone; clarified dev/packaged/mobile ownership and offline builds, and updated launch acceptance/milestones. **Documentation only; all milestones remain Planned.**
- **2026-10-25 — v2.4:** removed API authentication gates from endpoints, readiness and launch workflows; restricted unauthenticated operation to local loopback/development bridges, deferred public hosting and remote mobile connectivity, and updated milestones/ADR. **Documentation only; all milestones remain Planned.**

- **2026-10-25 — v2.5:** re-audited `main` at `1c847e8`; updated CLI library entry points, bounded Intel/topology and event contracts, Rust performance gates, canonical wiki/source references and repository-native planning workflow. No API/client stack or launcher implementation exists yet; milestones remain **Planned**. Documentation only.
- **2026-10-25 — v2.6:** Phase 1 web/API/launcher implemented and locally verified (embedded Rust gate, browser E2E, process smoke). Native phases remain Planned. Hosted CI not claimed.

## 13. Reference documentation

**Repository:** [README](https://github.com/xxxcess/argos-osint/blob/main/README.md) · [Cargo workspace](https://github.com/xxxcess/argos-osint/blob/main/Cargo.toml) · [Intel usage](https://github.com/xxxcess/argos-osint/blob/main/docs/wiki/concepts/usage.md#intel) · [Agent guidance](https://github.com/xxxcess/argos-osint/blob/main/AGENTS.md)

**CLI and process tooling:** [Clap derive](https://docs.rs/clap/latest/clap/_derive/) · [Next CLI](https://nextjs.org/docs/app/api-reference/cli/next) · [Tauri CLI](https://v2.tauri.app/reference/cli/) · [Tauri sidecars](https://v2.tauri.app/develop/sidecar/) · [Expo CLI](https://docs.expo.dev/more/expo-cli/)

**Current Argos implementation sources:** [Wiki entry](https://github.com/xxxcess/argos-osint/blob/main/docs/index.md) · [Core map](https://github.com/xxxcess/argos-osint/blob/main/docs/wiki/code/core.md) · [Interface map](https://github.com/xxxcess/argos-osint/blob/main/docs/wiki/code/interfaces.md) · [Storage map](https://github.com/xxxcess/argos-osint/blob/main/docs/wiki/code/storage.md) · [Planning format](https://github.com/xxxcess/argos-osint/blob/main/docs/wiki/code/planning-artifacts.md) · [Wiki maintenance](https://github.com/xxxcess/argos-osint/blob/main/docs/wiki/maintenance.md) · [TUI verification](https://github.com/xxxcess/argos-osint/blob/main/docs/wiki/code/tui-verification.md)

**Frameworks and builds:** [Next static exports](https://nextjs.org/docs/app/guides/static-exports) · [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) · [Tauri sidecars](https://v2.tauri.app/develop/sidecar/) · [Tauri macOS bundles](https://v2.tauri.app/distribute/macos-application-bundle/) · [Tauri Windows installer](https://v2.tauri.app/distribute/windows-installer/) · [Expo setup](https://docs.expo.dev/get-started/set-up-your-environment/) · [Expo monorepos](https://docs.expo.dev/guides/monorepos/) · [Expo APK builds](https://docs.expo.dev/build-reference/apk/) · [pnpm](https://pnpm.io/installation/)

## Phase 1 implementation record (2026-10-25)

Working branch: local `main`, including fetched `origin/main` merged at `b001ff7` before final verification. The existing feature-branch baseline was preserved; no remote push is part of this work. See [the maintained web/API contract](wiki/code/intel-web.md) for source boundaries, endpoint DTOs, launch commands, safety restrictions and verification entry points.

Dependency compatibility spike: Expo 57.0.31 `bundledNativeModules.json` specifies React/DOM 19.2.3 and React Native 0.86.3; these exact shared pins are used with RN Web 0.21.4 and Next 16.4.3. Node 24.22.1, pnpm 12.15.0, TypeScript 5.9.3, TanStack Query 5.104.5 and Playwright 1.64.1 are pinned. React Compiler remains disabled. This metadata/peer compatibility check and the web build do not verify Expo native builds, Tauri, macOS, Android, iOS or Windows.

Implementation choices: the web article endpoint pages stable run/article occurrences; the existing TUI keeps its current list semantics. The API uses repository brief/Related/Jobs/topology functions and never exports TUI App/cache state. API/Web stacks default to terminal logs; `--terminal tui` shares an owned in-process API repository with the normal TUI. Rust API and frontend traffic then appear as service entries in Rust Jobs → View logs; bounded, sanitized frontend output is also retained in per-home `stack-services.log`. The React Native Intel client has no operational Jobs/Logs connection. Adopted APIs require local server-mode storage for concurrent TUI display. One foreground supervisor is managed per Argos home. Independently running compatible loopback APIs can be adopted; an already managed stack must be stopped before changing its target. Stale supervisor state fails closed. Public deployment, authentication and all native phases remain deferred. The user authorized the new `.github/workflows/intel-web.yml` as a scoped ownership exception, with Code Architect review retained. Existing Rust workflows are unchanged.

Frontend verification: `pnpm install --frozen-lockfile`, `pnpm peers check`, `pnpm typecheck`, `pnpm lint`, `pnpm build`, and `pnpm test:e2e` pass on the pinned toolchain. Browser E2E reports 4/4 passing and checks offline gating, six categories, search, full retained body, ensure/reload, topology, reconnection, narrow tabs and static-host CSP. The 2026-10-26 UI scope update also asserts no operational Jobs/Logs/job-stage requests. Wide Bulletin/Focus Brief, narrow Context and lifecycle-diagram captures are retained in the named plan. The static export is `apps/web-desktop/out/`. Next webpack aliases `react-native` to the web-desktop `react-native-web` module so `next dev` matches the export.

Rust/API verification (local, 2026-10-25, macOS x86_64, Rust 1.95.0): `scripts/local_matrix.sh --storage embedded` completed green (`test-surreal` embedded 521s) after isolating `ARGOS_HOME` in `storage_brain_scope` and the Profile config export unit test. Ambient developer settings were causing those two existing tests to leave the offline path.

Process smoke: `python3 scripts/agent_cargo.py build --locked -p argos-osint-bin -p argos-api-host` then `python3 scripts/test_web_stack.py` PASS. The launcher now requires `storage/config.json` or `ARGOS_STORAGE_MODE` before starting an owned API (`load_config`’s implicit embedded default is not treated as initialized). Smoke covers exclusive storage, installed static hosting, Next dev after API readiness, SIGINT cleanup, API adoption retention, and refusal when the adopted API is down. Hosted GitHub Actions were not run.

CI ownership decision: the user requested both applying the prepared workflow and retaining Code Architect review ([review handoff](../.planning/2026-10-25-argos-intel/handoff.md)). `intel-web.yml` is therefore included; it does not deploy, require live API/provider keys, or replace the Rust matrix. YAML syntax and frontend commands are locally verified; hosted Actions results remain unavailable until the change is published.

Terminal-display follow-up (2026-10-26): `--terminal tui|logs` and `--no-open` are implemented. The final welcome question selects TUI/logs/cancel. Owned API/TUI stacks share one in-process repository and elected workers; the TUI boots before entering the existing runtime handle and Brain refreshes use background snapshots. Web launch waits for HTTP readiness before invoking the OS default-browser URL handler. Local verification: bin unit suite 267 passed / 6 ignored, CLI integration 4/4, locked binary build and bin all-targets clippy pass. Process checks cover browser dispatch/suppression and normal log-mode lifecycles; final targeted Unix PTY checks verify shared embedded API/Web TUI, Brain navigation, saved-session dismissal, Ctrl-Q/down cleanup and terminal flag restoration. Actual 120×40 captures and the updated lifecycle diagram were inspected. The earlier Phase 1 full embedded gate remains recorded above; this follow-up used focused checks.

Node selection follow-up (2026-10-26): `.nvmrc` pins 24.22.1 alongside `.node-version`. Web launch, build and doctor now invoke `nvm use` and reuse its environment for Node/pnpm; they do not depend on the calling shell already selecting Node24. Verified three targeted nvm tests, locked binary build, all-targets bin clippy, full process smoke, doctor and static web build from a Node25.8.0 caller. Child tools selected Node24.22.1/pnpm12.15.0; the calling shell remained on25.8.0.

Single-binary launch follow-up (2026-10-26): managed log-mode API startup now uses an internal host mode in a child copy of the current `argos` executable. `cargo run -p argos-osint-bin` no longer requires a separately built sibling `argos-api-host`; that binary remains optional for standalone use/adoption. API router, loopback/Origin restrictions, repository ownership, workers, readiness and lifecycle guarantees are preserved. Verified CLI integration 4/4, locked build, all-targets bin clippy and full process smoke, including API/Web startup from an isolated executable directory containing only `argos`.

ARGOS VENTURE follow-up (2026-10-26): the no-argument launcher now uses Ratatui, the core TUI's wordmark stroke renderer/theme and centered configuration components; its title is ARGOS VENTURE. Main Home remains ARGOS OSINT. Terminal TUI mode exposes Rust API and Next/static service jobs through the existing Rust Jobs → View logs flow. Traffic ingestion is bounded and strips secrets, control sequences and query strings; stale service-owner recovery is scoped to these services. React Native Intel navigation contains Bulletin/Topology only and makes no operational Jobs/Logs/stage requests. Seven launcher interaction tests, 35 scoped TestBackend captures with representative PNG inspection, frontend typecheck/lint/build and browser E2E 4/4 pass. Full process smoke passes the wizard, service traffic, browser dispatch, single-binary startup, embedded ownership, adoption and terminal restoration. Live PTY logs were rendered and reviewed; the empty-string React Native View warning found there is fixed and guarded against. The final full embedded matrix is green: fmt, workspace clippy, workspace tests (604s), storage CLI smoke and embedded database harness (572s). Server CI and hosted Actions were not run locally.

Article media follow-up (2026-10-26): removed core TUI article image rendering, fetch/decode state, protocol detection and image dependencies. Bulletin and Focus Brief use full-width text; image URL metadata remains available to clients. React Native category-feed cards display images left of the text at wide and compact sizes, collapsing missing/unsafe/failed sources. Verified frontend typecheck/lint/export and browser E2E 9/9, bin units 266 passed/6 ignored, full embedded matrix green (database harness501s), twelve TestBackend captures with representative PNG review, and live API/installed-Web TUI with no image graphics/DSR probe and clean terminal restoration. A separate Next dev smoke attempt encountered an existing user's dev-server lock; that server was preserved and installed-Web verification passed. Capture tooling now matches workspace library feature unification to reuse gate artifacts. Hosted CI and live-provider images were not exercised.
