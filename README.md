# Argos OSINT

Argos is a terminal investigation workspace. It gathers public sources, binds evidence, and writes cited answers.

Home launches four applications — **Intel**, **Atlas**, **Brain**, **Recon** — and five system surfaces: **Jobs**, **Logs**, **Tools**, **Models**, **Profile**.

Only Recon has a chat. Everything else is fields, lists, and buttons. Observations are retrieved data with a timestamp. They are not proof of identity or ownership.

[Docs](docs/README.md) · [Usage](docs/wiki/concepts/usage.md) · [Figures](docs/diagrams.md) · [Releases](docs/wiki/code/versioning.md) · [Changelog](CHANGELOG.md)

[![Argos workspace](docs/diagrams/workspace.svg)](docs/diagrams/workspace.html)

## Tech stack

| Layer | Current implementation |
| --- | --- |
| CLI and core | Rust 1.95.0, Tokio, Clap; shared core plus CLI/TUI and local API crates |
| Terminal UI | Ratatui 0.30 and Crossterm 0.28; shared theme, measured keyboard/mouse targets and deterministic terminal fixtures |
| Persistence | SurrealDB 3.3.4; default in-process RocksDB storage, optional loopback SurrealDB server mode |
| Local HTTP API | Axum 0.8 with typed `/api/v1` projections and storage-backed readiness; Rust owns data and workers |
| Web UI | React 19.2.3, React Native 0.86.3 primitives mapped through React Native Web 0.21.4; Next.js 16.4.3 static export and development server |
| React compilation and state | React Compiler 1.0.0; TanStack Query 5.104.5 for async/cache/cursor state, Context for connection/preferences, app adapters for browser settings |
| JavaScript toolchain | Node 24.22.1, pnpm 12.15.0, TypeScript 5.9.3; pinned workspace manifests and frozen lockfile |
| Providers and local NLP | Reqwest/rustls, QuickJS via rquickjs, tokenizers and tract-onnx; live provider/model execution is separate from offline tests |
| Quality and CI | rustfmt, Clippy, Cargo/storage/process tests; ESLint 9 with Hooks/compiler rules; Playwright 1.64.1 export/dev browser fixtures; GitHub Actions |

Versions and feature ownership are defined in [Cargo.toml](Cargo.toml), the [frontend manifests](package.json) and [web app manifest](apps/web-desktop/package.json). See the [architecture](docs/wiki/code/architecture.md), [React UI contract](docs/wiki/code/react-ui.md) and [CI guide](docs/wiki/code/ci.md). Expo mobile apps, Tauri desktop bundles and native-platform validation remain planned; the current native primitives do not imply shipped native clients.

## Applications

| App | What it does | Read more |
| --- | --- | --- |
| Intel | Browse Atlas headlines and prepare a cited Focus Brief with ingest-related articles | [Usage](docs/wiki/concepts/usage.md#intel) · [Brief](docs/diagrams/intel-brief-chain.html) |
| Atlas | Classifier-only news collection, country heat, local metadata and Related leads | [Usage](docs/wiki/concepts/usage.md#atlas) · [Cycle](docs/diagrams/atlas-pipeline.html) |
| Brain | Claims, unique Actor names and navigable associations with read-only graphs | [Usage](docs/wiki/concepts/usage.md#brain) · [Browse](docs/diagrams/brain-browse.html) · [Recall](docs/diagrams/brain-recall.html) |
| Recon | Evidence-driven investigations with a planner, binder, and synthesis | [Usage](docs/wiki/concepts/usage.md#recon) · [Architecture](docs/wiki/code/architecture.md) · [Turn](docs/diagrams/recon-turn.html) |
| Tools | Manual OSINT catalog (internal id `Osint`) | [Providers](docs/wiki/concepts/providers.md) · [Catalog](docs/diagrams/osint-providers.html) |
| Models | Role defaults and OpenRouter / Google / Nvidia accounts (`Providers`) | [Providers](docs/wiki/concepts/providers.md) · [Roles](docs/diagrams/model-roles.html) |
| Jobs / Logs | Background work and durable events | [Usage](docs/wiki/concepts/usage.md#other-apps) · [Jobs](docs/diagrams/jobs-lifecycle.html) |
| Profile | Host hardware and storage paths (`System`) | [Concepts](docs/wiki/concepts/glossary.md) |

## Quick start

Rust **1.95.0** is pinned. The first build needs a C toolchain (`xcode-select --install` on macOS). Protoc is vendored. Run these commands from the repository root.

### Terminal app

```sh
cargo run --locked -p argos-osint-bin
# Or build once and open the service menu:
cargo build --locked -p argos-osint-bin
./target/debug/argos
# Skip the menu and open the TUI:
./target/debug/argos stack up --tui
```

Running `argos` without arguments opens a Ratatui service launcher with a centered ARGOS VENTURE banner and the same theme as the main TUI. Use ↑/↓, Tab or the numbered choices, then Enter; mouse clicks also select choices. Settings cover storage mode, API/web ports, the web profile, and starting or connecting to an API. Enter accepts a default, Ctrl+U clears a text field, and Esc returns to the previous question with its saved value (Esc at the service root exits). Ctrl+Q exits the wizard. PgUp/PgDn review the selected settings. The final choice starts the stack with either the normal TUI or server logs visible. When a session launched from Venture ends, its owned services stop and Venture reopens. Explicit `stack up` commands remain one-shot.

Explicit `stack up` commands skip the launcher. The main TUI opens on Home: ↑/↓ and Enter open an app, keys `1`–`9` jump between apps, Esc returns Home, and `?` opens shortcuts. API Server and Web use the build prerequisites below.

A default build uses embedded SurrealDB: RocksDB on disk under `~/.argos/storage/surreal` (`ARGOS_HOME` moves it), in-process. To use a per-home SurrealDB 3.3.4 server on loopback, build with `--no-default-features --features server,nlp` and select `argos storage init --mode server` (or `ARGOS_STORAGE_MODE=server`).

### Intel web development

Use Node **24.22.1** and pnpm **12.15.0**. The repository's `.nvmrc` pins Node for nvm. Web launch, build and doctor commands automatically run `nvm use` and pass the selected environment to Node/pnpm. Install that version once with nvm; for direct `pnpm` commands, select it in your shell:

```sh
nvm install
nvm use
node --version  # v24.22.1
```

Build `argos`. The launcher starts a child copy of the same binary in its built-in API-host mode; a separate `argos-api-host` binary is optional.

```sh
pnpm install --frozen-lockfile
cargo build --locked -p argos-osint-bin
# First use of a new home: choose storage explicitly.
./target/debug/argos storage init --mode embedded
./target/debug/argos stack doctor --target web
./target/debug/argos stack up --web --port 3000 --api-port 8080
```

The foreground launcher starts the loopback API, waits for storage-backed readiness, then starts Next development mode. Once the web server responds, it opens the printed URL in your default browser. Use `--no-open` to keep the browser closed. For an existing home, keep its storage mode; owned API launches require `storage/config.json` or an explicit `ARGOS_STORAGE_MODE`.

### Build and serve the web export

After installing dependencies and building Argos above:

```sh
./target/debug/argos stack build --web
# Equivalent frontend-only build: pnpm build
./target/debug/argos stack up --web --profile installed --port 3000
```

The build writes `apps/web-desktop/out/` without needing a running API. The installed profile serves that export with Node and starts or adopts the API before serving the UI. It uses the repository's static host. For an optimized Rust binary, build with `cargo build --release --locked -p argos-osint-bin`, then use `./target/release/argos` in the commands above.

To keep the normal TUI visible while API/Web services run:

```sh
./target/debug/argos stack up --web --terminal tui
# Or run the API with the TUI in this terminal:
./target/debug/argos stack up --server --api-port 8080 --terminal tui
# Keep API and Next/static-host logs visible instead (the default):
./target/debug/argos stack up --web --terminal logs
```

With an owned API and `--terminal tui`, the API and TUI share one repository in the same process, including embedded storage. Open **Jobs**, select **Rust API server**, then **View logs** (`l`) for request method, route, status and duration. Web stacks also have a **Next.js web server** job (or **Static web server** for the built export) with access logs and server output. The frontend also writes redacted output to `ARGOS_HOME/stack-services.log` (by default `~/.argos/stack-services.log`). Quitting the TUI stops the managed stack. An independently adopted API needs this home's server-mode storage to display the TUI concurrently; log mode works with an embedded API.

The Intel web app contains Bulletin, Focus Brief, Related and topology. Jobs and Logs are Rust system apps; React Native clients do not connect to operational Jobs/Logs or job-stage endpoints.

### Stack lifecycle and API only

`up` stays in the foreground. Use Ctrl-C to stop it, or run these from another terminal with the same `ARGOS_HOME`:

```sh
./target/debug/argos stack status
./target/debug/argos stack down --web
# After stopping the web stack, run only the HTTP API:
./target/debug/argos stack up --server --api-port 8080
# Separate terminal:
./target/debug/argos stack down --server
```

Here `--server` selects the HTTP API; storage mode is configured separately. One managed stack runs per Argos home. Embedded storage has one owner, so stop a separately running TUI before launching an API against the same home.

To connect the web app to an independently running API, start the host in one terminal, then launch the web app in another:

```sh
# Optional standalone host (the managed launcher does not need this binary):
cargo build --locked -p argos-api-host
./target/debug/argos-api-host --port 8080
# Separate terminal:
./target/debug/argos stack up --web --api-url http://127.0.0.1:8080 --port 3000
```

Stopping this web stack leaves the adopted API running. For a different web port, configure the standalone host's `--origins` allowlist accordingly. All API/web hosting is local loopback. macOS, Android, iOS and Windows stack targets remain planned.

See the [web/API guide](docs/wiki/code/intel-web.md) for backend selection, storage ownership and endpoint contracts, and the [living implementation specification](docs/argos-intel-living-spec.md) for verified milestones and the [branch-to-main delivery scope](docs/argos-intel-living-spec.md#branch-to-main-scope-2026-10-26), including the minimalist web UI based on the core TUI.

### Development checks

Shared UI uses React Native primitives mapped to React Native Web. React Compiler is enabled for Next and the shared packages; lint includes Hooks/compiler diagnostics with zero warnings. Keep both integrations enabled. [React UI rules](docs/wiki/code/react-ui.md#react-native-and-compiler-baseline) describe the platform boundary and verification requirements.

```sh
# Targeted local Rust checks & tests (fast iteration on the host machine; embedded storage mode)
cargo check --locked -p argos-osint-core                  # or -p argos-osint-bin | -p argos-osint-api | -p argos-api-host
cargo clippy --locked -p argos-osint-core --all-targets -- -D warnings
cargo test --locked -p argos-osint-core --lib <filter>    # scoped unit tests
cargo test --locked -p argos-osint-api --test api_contract
ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 \
  cargo test --locked -p argos-osint-core --test storage_jobs <filter> -- --nocapture

# Full workspace & embedded storage gate (agents use `python3 scripts/agent_cargo.py … --locked`)
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked  # offline; ARGOS_EMBED unset
ARGOS_EMBED=1 cargo test --locked -p argos-osint-core -- --ignored minilm
scripts/local_matrix.sh --storage embedded

# Web & E2E checks
pnpm typecheck
pnpm lint
pnpm build
pnpm check:web-build
pnpm exec playwright install chromium
pnpm test:e2e
ARGOS_WEB_PROFILE=dev pnpm test:e2e  # Also verify Next development compilation
```

Navigation, CLI, and state directory: [usage guide](docs/wiki/concepts/usage.md). Contributors and agents should follow the [contributor workflow](docs/wiki/code/contributing.md), including the agent Cargo wrapper and CI-only server storage gate. Agent guidance: [AGENTS.md](AGENTS.md).

### Agent & CLI workflows (OpenCode & Google Antigravity CLI)

Contributors working with agentic coding CLIs follow the shared repository standards in [AGENTS.md](AGENTS.md), [contributor workflow](docs/wiki/code/contributing.md), and [planning artifacts](docs/wiki/code/planning-artifacts.md):

- **Workspace & persistence standards:**
  - **Temporary files & worktrees (`.agent-scratch/`):** Keep all scratch files, patches, diagnostic logs, isolated build caches, and feature worktrees (`.agent-scratch/worktrees/`) inside `.agent-scratch/` (unstaged and never committed).
  - **Long-term planning (`.planning/`):** Record multi-phase plans, design specs, and decision ledgers in `.planning/<YYYY-MM-DD-slug>/` following the [planning format](docs/wiki/code/planning-artifacts.md#working-format).
  - **Rust embedded storage, host-native platform & targeted builds (see `.github/workflows/ci.yml`):** Local **Rust** builds and tests run strictly in **embedded storage mode** (`--storage embedded`, default features `embedded,nlp`; `server` mode is CI-only) and target the **local host machine platform** (inspectable via bash shell commands such as `uname -ms` and `rustc -vV | grep '^host:'`). Prefer targeted crate and test commands (`python3 scripts/agent_cargo.py check|clippy|test --locked -p <crate> [--lib | --bin <bin> | --test <name>]`, and `ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 python3 scripts/agent_cargo.py test --locked -p argos-osint-core --test storage_<name>`) during scoped iteration before running `scripts/local_matrix.sh --storage embedded`. Reference [`.github/workflows/ci.yml`](.github/workflows/ci.yml), [`.github/workflows/intel-web.yml`](.github/workflows/intel-web.yml), [`.github/workflows/commit-policy.yml`](.github/workflows/commit-policy.yml), and the [CI workflow guide](docs/wiki/code/ci.md), keep React Compiler/Hooks lint at zero warnings, and update affected `docs/wiki/` routes, `@tag:<slug>` entries, and diagrams in the same change.
- **OpenCode setup (`.opencode/`):**
  - Configured in [.opencode/opencode.json](.opencode/opencode.json) and [.opencode/AGENTS.md](.opencode/AGENTS.md) with the 4-agent team (`architect`, `coder`, `tester`, `scribe`), worker model overrides (`.opencode/architect-worker.json`, `.opencode/coder-worker.json`), and repository skills (`.opencode/skills/`).
  - Follow the [OpenCode agent team guide](docs/wiki/code/opencode-v2.md) for role permissions, phase handoff envelopes, disjoint `EDIT_UNIT` worktree integration, and Scribe documentation checkpoints.
- **Google Antigravity CLI `/teamwork` guidance (`.agy/`):**
  - Users running the `/teamwork` command in Google Antigravity CLI should reference [.agy/howto.md](.agy/howto.md) to select the right workflow spec for their task scope:
    - **Lite Team ([.agy/lite-team.md](.agy/lite-team.md)):** Fast-path 2-agent workflow (`coder` + `tester`) for scoped fixes and single-module updates ($\le 3$ files, no breaking contract/schema changes).
    - **Heavy Team ([.agy/heavy-team.md](.agy/heavy-team.md)):** Full 4-agent orchestration (`architect`, `coder`, `tester`, `scribe`) with Phase 0 `.planning/` bootstrap, parallel `.agent-scratch/worktrees/` execution, and full host-native verification gates.
    - **Scope-specific React Native UI `tester` (`anthropic/claude-sonnet-5.5#low`):** Both specs dispatch a dedicated `tester` agent on `anthropic/claude-sonnet-5.5#low` for React Native UI scope (`apps/web-desktop`, `packages/{ui,intel,api-client}`) to enforce repository design standards and conventions ([React UI rules](docs/wiki/code/react-ui.md), [conventions](docs/wiki/code/conventions.md), `theme.rs` HUD/glass palette, React Compiler/Hooks lint, and Playwright fixture screenshot review), alongside `google/argon#low` for Rust/system verification.

## Figures

Open in a browser for the HTML page, or view the SVG inline.

[![One Recon turn](docs/diagrams/recon-turn.svg)](docs/diagrams/recon-turn.html)

[![Persistence stack](docs/diagrams/persistence.svg)](docs/diagrams/persistence.html)

[![Intel brief chain](docs/diagrams/intel-brief-chain.svg)](docs/diagrams/intel-brief-chain.html)

The rest of the set: [docs/diagrams.md](docs/diagrams.md).

## Documentation

Start at the [repo wiki](docs/index.md): choose [concepts](docs/wiki/concepts/index.md) to understand Argos or [code](docs/wiki/code/index.md) to explore its implementation. Agent navigation starts at [AGENTS.md](AGENTS.md).

## Limits

- Adapters issue bounded public HTTP. A catalog entry or fixture test does not prove live availability.
- Shodan InternetDB is noncommercial.
- Nominatim needs an identifying User-Agent, attribution, caching, and at most one request per second.
- Model calls use the account and terms you configure.
