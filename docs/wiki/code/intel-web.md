# Local Intel web application

[Code route](index.md) · [Usage](../concepts/usage.md) · [Living specification](../../argos-intel-living-spec.md)

`@tag:intel-brief`

[![Intel web lifecycle](../../diagrams/intel-web.svg)](../../diagrams/intel-web.html)

The Next static export is a client of the Rust HTTP API. It never opens SurrealDB or receives provider credentials. In log mode, the launcher starts a child copy of `argos` in a hidden API-host mode; that child owns the repository and elected core workers/heartbeat. A separate `argos-api-host` binary is optional for standalone use/adoption. With an owned API and `--terminal tui`, the supervisor hosts the same API router in-process and the TUI shares its single runtime repository; this supports embedded storage without a second owner. The in-process host starts the workers/heartbeat before readiness; the TUI reuses those workers. The process heartbeat runs every 30 seconds. An owner with no beat for 90 seconds is treated as gone. Standalone `stack up --tui` continues directly to core/storage without an HTTP listener.

## Source boundaries

| Layer | Source | Responsibility |
| --- | --- | --- |
| Core | [intel_service.rs](../../../crates/argos-osint-core/src/intel_service.rs) | Bounded article occurrence pages, immutable brief projections, ensure/reload admission, Related |
| HTTP | [argos-osint-api](../../../crates/argos-osint-api/src/lib.rs) | `/api/v1`, DTO validation, storage readiness, Host/Origin/CORS |
| Host | [argos-api-host](../../../crates/argos-api-host/src/main.rs) | Optional standalone loopback listener, repository ownership, brief worker, shutdown |
| Supervisor | [stack](../../../crates/argos-osint-bin/src/stack/mod.rs) | Preflight, owned/adopted API, readiness-before-frontend, live control channel, cleanup |
| Presentation | [presentation.rs](../../../crates/argos-osint-bin/src/stack/presentation.rs), [node.rs](../../../crates/argos-osint-bin/src/stack/node.rs) | Shared-process API, terminal lifecycle, nvm environment, web HTTP readiness and OS browser handler |
| Client | [api-client](../../../packages/api-client/src/index.tsx) | Context and TanStack Query v5, backend/instance cache namespaces, readiness polling |
| UI | [intel](../../../packages/intel/src/index.tsx), [ui](../../../packages/ui/src/index.tsx) | Bulletin, Focus Brief, Related leads and bounded topology navigation; operational Jobs/Logs remain in Rust |
| Service logs | [traffic.rs](../../../crates/argos-osint-bin/src/stack/traffic.rs) | API access events and frontend output attached to existing Jobs/Logs service rows |
| Static host | [serve_web.mjs](../../../scripts/serve_web.mjs) | Loopback export hosting, CSP and Host restrictions |

## Feed imagery

React Native Intel Bulletin cards show a cover-cropped article thumbnail to the left of the main text container. Featured/regular thumbnails use 240×180 / 160×120 pixels on wide viewports; below 600 pixels they use 72×88 pixels with tighter card spacing. The text container can shrink and wrap without horizontal page overflow. Missing/unsafe image URLs and load failures collapse the thumbnail, preserving the article action. Images load directly in the client under the existing image CSP; the API retains image URL metadata. The core terminal TUI has no image renderer, image fetch/decode work or terminal graphics probe.

## API contract

All application endpoints are unauthenticated. The host binds **only `127.0.0.1`**. Host must equal `127.0.0.1:PORT` or `localhost:PORT` (HTTP port 80 may omit its default port). Origins are normalized to the browser’s canonical origin before allowlisting. An Origin, when supplied, must exactly match the configured loopback web origin; rejected Origins receive no CORS permission. Requests without Origin support local CLI clients. This is a trusted local development interface, not remote access protection.

| Method / path (under `/api/v1`) | Input / result |
| --- | --- |
| GET `health/ready` | Live schema read; service/version/instance, storage/ready booleans; 503 when schema/storage fails |
| GET `capabilities` | API version, six categories and supported features |
| GET `days` | Retained Atlas run days and categories |
| GET `articles` | `category`, `day`, optional `search`, `cursor`, `limit` (1–100, default 30); rows/revision/next_cursor |
| GET `articles/detail` | `run`, `id`; retained body, actors, claims, lifecycle, publication, latest job and indexing receipt |
| GET `articles/related` | `run`, `id`, optional `cursor`, `limit` (1–100); related leads with basis and tentative state |
| POST `brief/ensure` | `{run,id}`; attach/admit one chain for unchanged inputs; `{job_id}` |
| POST `brief/reload` | `{run,id,request_id}`; explicit new generation; retrying the same request ID attaches to that generation |
| GET `brief/stages` | `id` job; stage views and committed outputs |
| GET `jobs` | Bounded 100 recent/active jobs |
| GET `jobs/detail` | `id`; job and its brief stages |
| POST `jobs/retry` | `{id}`; existing safe failed-task requeue; completed tasks are untouched |
| POST `topology` | `{seed}` using core `TopologySeed`; default bounded core topology, navigable node seeds |

Article lists use keyset pagination over **occurrences** (run/article pairs), ordered by published time descending, seen time descending, article ID then run ID. They filter run days in one query and never loop through `list_runs`/`list_articles`. The additive `atlas_article_bulletin` index is bootstrapped idempotently without changing record formats or schema version. Search is scoped to a day/category and limited to 256 bytes. Cursors are bounded JSON tuples and must be treated as opaque by clients. Brief DTO `revision` hashes display content, while `source_revision` is the core article metadata hash used to fence topology seeds. Metadata actors and unbound claims use retained source seeds, preserving their provisional status. Revisions hash result content; they do not advance merely because a client polls.

Opening a Focus Brief automatically calls `brief/ensure` for its run/article occurrence, including navigation from Related. Admission uses the same core chain contract as the TUI: unchanged inputs attach to existing work. The client admits once per mounted occurrence/backend session; detail polling and refresh do not repeatedly submit it. Reopening safely reattaches; explicit Reload still creates a new generation. Admission failures are visible above the brief on all layouts, with Ensure / resume available for retry in Context.

Brief generations and canonical publication are separate states. The DTO includes the latest job and a matching canonical indexing receipt when available; absence is shown as not indexed. Related articles are leads, never proof of corroboration. Topology navigation does not write memories; canonical publication belongs to core brief jobs. The narrow Focus Brief uses Extracted/Reader/Context tabs; wide layout allocates approximately 24/48/28 percent.

## Running locally

Use Rust 1.95.0, nvm with Node **24.22.1** installed, and pnpm **12.15.0** available under that Node version. Toolchain installation is a separate user action; `doctor` checks tools without installing them. JavaScript versions and peer compatibility are pinned in the workspace manifests and lockfile. React Compiler remains disabled. The shared React 19.2.3 / RN 0.86.3 / RN Web 0.21.4 pins match the bundled React/RN versions in Expo 57.0.31 metadata; this is a dependency compatibility spike, not a native build. Native Expo/Tauri compatibility and builds remain later phases.

```sh
# Select .nvmrc in this shell before launching Argos (install once if needed):
nvm install
nvm use
pnpm install --frozen-lockfile
python3 scripts/agent_cargo.py build --locked -p argos-osint-bin
# First use only: select storage explicitly; do not change an existing home's mode.
target/agents/debug/argos storage init --mode embedded
# No arguments opens the TUI / API Server / Web service menu.
# To enter the TUI directly: target/agents/debug/argos stack up --tui
target/agents/debug/argos stack doctor --target web
target/agents/debug/argos stack up --web --port 3000
# Separate terminal:
target/agents/debug/argos stack status
target/agents/debug/argos stack down --web

# API only; --server means HTTP API, not SurrealDB server:
target/agents/debug/argos stack up --server --api-port 8080
# Or build the optional standalone host and run it independently:
# python3 scripts/agent_cargo.py build --locked -p argos-api-host
# target/agents/debug/argos-api-host --port 8080
target/agents/debug/argos stack up --web --api-url http://127.0.0.1:8080

pnpm --dir apps/web-desktop build
# Installed web profile serves out/, never next start:
target/agents/debug/argos stack up --web --profile installed
```

The no-argument welcome menu gathers these settings before dispatching the same `stack up` commands:

| Setting | When asked | Default |
| --- | --- | --- |
| Storage mode | TUI or a locally owned API | Keep configured mode; embedded for a new default-build home |
| SurrealDB storage port | New server-mode home | Auto-selected; distinct from API/web ports |
| HTTP API port | API Server or Web with a locally owned API | 8080 |
| Web port | Web | 3000 |
| Web profile | Web | `dev`; `installed` serves a previously built export |
| Start/connect API | Web | Start locally; connect asks for a loopback URL |
| Terminal display (last question) | API Server / Web | `logs`; choose `tui` to keep the normal TUI visible, or `cancel` to exit |

The no-argument launcher is a Ratatui wizard sharing the main TUI's semantic colors, panels and focus styles. Keyboard and mouse use the same choice rectangles. ↑/↓, Tab or a number selects a choice; Enter continues; Esc cancels. Text fields support cursor movement, Unicode editing, paste and Ctrl+U clear. PgUp/PgDn review the accumulated settings and home. Below 40×14 the launcher asks for a resize. Redirected stdin/stdout retains text prompts for scripts. Existing storage modes are preserved; changing an initialized home requires the explicit storage workflow. New-home storage initialization runs as a separate `argos storage init` process, releasing its datastore handles before TUI/API startup. An adopted API manages its own storage. Server storage needs the pinned SurrealDB binary (`argos storage install`); build/export prerequisites still apply.

Web `up` (dev and installed), `build --web` and `doctor --target web` source nvm and run `nvm use` for the exact version in `.nvmrc`. The resulting Node binary and PATH are used for all Node/pnpm checks and frontend processes, even if the caller uses Node 25. The caller's shell and nvm default alias are not changed by the launcher. nvm is found at `$NVM_DIR/nvm.sh`, `~/.nvm/nvm.sh`, or the standard Homebrew nvm locations. Missing nvm or a missing pinned Node version produces an installation instruction; launch does not install tools. Standalone TUI and API Server do not need Node or nvm.

Owned API/web launches require an explicit storage choice: `argos storage init` (`storage/config.json`) or `ARGOS_STORAGE_MODE`. The default embedded build still implies a mode through `load_config`; the launcher does not treat that implicit default as initialized. Next webpack aliases `react-native` to the web-desktop `react-native-web` install so `stack up --web` (dev) and the static export resolve the same UI.

Use `--terminal tui` on `stack up --server` or `stack up --web` for the normal TUI while services run; `--terminal logs` (the explicit CLI default) inherits API and Next/static-host output. The TUI boots on a plain terminal thread, then enters the existing Tokio handle for normal async actions. Repository-driven Brain refreshes use background snapshots. Ctrl-Q, `stack down`, service failure and process signals stop owned services and restore terminal mode. An adopted API cannot share another process's embedded handle: TUI display with adoption requires local server storage, while owned APIs support both storage modes.

### Traffic in Rust Jobs / Logs

Owned API hosts register a **Rust API server** service job. Open Jobs, select it and activate **View logs** (`l`) to see request method, matched route, response status and elapsed milliseconds. Query strings, request headers and bodies are excluded. In a Web stack with TUI display, **Next.js web server** (dev) or **Static web server** (installed) provides the frontend access logs and stdout/stderr. The built static host logs method/path/status/duration too. Frontend output is stripped of terminal escapes, redacted and also appended to the private per-home `stack-services.log`.

Both producers write through the existing in-process repository. A 256-entry queue per service decouples requests/pipes from persistence; input lines are capped at 4 KiB, display text is bounded, and overflow is reported with an omitted-entry count. Service rows do not offer task retry or cancellation; stop them through the stack lifecycle. Clean shutdown completes them, unexpected exits fail them, and the process registry recovers API/web service rows belonging to dead owners. Existing event retention and job filters apply.

Running `api_server` and `web_server` service rows sort ahead of ordinary work so crowded history cannot push them out of the bounded Jobs/API page. Explicit status/app/search filters still apply; use All statuses and all apps to see both. Jobs selects the bounded page of IDs, then loads grouped event/task/attempt metrics for that page instead of running correlated detail scans per row. Event counts retain the root-plus-immediate-children cohort; attempt and start-time metrics match single-job detail. Service rows remain excluded from the work-count header; their attached events are available through View logs.

The React Native Intel package has no Jobs tab, operational job actions or job-stage polling. Brief lifecycle/publication/indexing remain article metadata. Operational HTTP endpoints remain available for local diagnostic clients, but the React Native UI does not consume them.

For web launches, the API passes readiness/Origin checks first, then the frontend must answer HTTP successfully. The launcher opens the URL with its selected `?api=` through the OS default-browser URL handler, which uses the browser's tab preferences. `--no-open` suppresses this; opener failure leaves services running and prints the manual URL. Repeating a matching web `up` can reopen the URL. Static hosting uses the same behavior; a dev compile may take up to the 120-second HTTP-readiness deadline.

`cargo run -p argos-osint-bin` is sufficient for the Rust launcher: managed API child processes use the current executable's internal host mode, with matching API code and Cargo features. They require no sibling host binary or runtime Cargo build. `doctor` reports this built-in mode. The optional standalone host remains supported for independently managed/adopted APIs. The process smoke test launches an isolated `argos` hard link with no companion binary to cover this path.

One supervisor is managed per Argos home. `up` runs in the foreground; its API must pass compatible storage-backed readiness and web Origin permission before Next or static hosting starts. It prints a URL containing the selected backend origin. `status`/`down` use the per-home live supervisor capability channel and instance identity; they never signal a PID loaded from disk. `down` requests the live owner to stop its held children; an adopted API remains running. An abandoned state file fails closed with a diagnostic instead of guessing which process to stop. Do not delete it until any former children have exited. Embedded storage keeps its existing exclusive home lock; simultaneous direct TUI/API processes require the supported SurrealDB server mode; the owned `--terminal tui` stack instead shares one process/repository. The launcher never changes that mode automatically.

Directly opened web clients default to `http://127.0.0.1:8080`, accept a loopback `?api=` parameter, and show a connect/retry screen until a compatible API is ready. An open client polls `health/ready` once a minute. Failed readiness polling removes the working UI and blocks mutations; recovery rebuilds queries under the API instance namespace. Static builds do not need an API.

The local static host sets a CSP that permits local scripts, hashed Next inline bootstrap (SHA-256 hashes) and RN Web inline styles, HTTPS article images and loopback API connections; it forbids frames, forms and remote scripts. Dev Next has its own development policy. Public deployment remains deferred.

## Verification

- [API contracts](../../../crates/argos-osint-api/tests/api_contract.rs): real temporary embedded SurrealDB, pagination/revisions, admission/reload idempotency, readiness, missing IDs and CORS/Host rejection.
- [Process smoke](../../../scripts/test_web_stack.py): `python3 scripts/test_web_stack.py` after building `argos`, the optional `argos-api-host` fixture and the static export; uses a temporary Argos home, a stub OS URL opener, and Unix PTYs to check browser dispatch, shared embedded ownership, no terminal article-image graphics probe and terminal restoration. For focused PTY checks while an existing Next dev server holds the workspace lock, use `--tui-only --tui-web-profile installed`; leave that existing server running. `--terminal-captures <directory>` retains PTY streams and bounded service logs for diagnostics.
- [Launcher parsing](../../../crates/argos-osint-bin/tests/stack_cli.rs): exclusive flags, unsupported phases, URL and port validation.
- [Browser E2E](../../../tests/web/intel.spec.ts): offline HTTP fixtures, left-side feed images at 1440/768/390/320px, missing/unsafe/failed images, browse/search, body, mutations, topology and reconnect at wide/narrow viewports; asserts no Jobs/Logs/job-stage requests.
- Launcher/traffic visuals: `scripts/tui_review.py capture --test launcher_cells_and_mouse_targets --include 'launcher-*'` and `--test service_jobs_and_traffic_screens --include 'service-*'` with a fresh named-plan output directory. Live process smoke exercises the wizard and Jobs → View logs with embedded API/Web stacks.
- `pnpm typecheck`, `pnpm lint`, `pnpm build`, `pnpm test:e2e`.
- Existing Rust regression gate: `scripts/local_matrix.sh --storage embedded`; server storage remains CI-only.

See the living spec milestone table for actual verification status and remaining acceptance gaps. The [web CI workflow](../../../.github/workflows/intel-web.yml) runs the frontend checks on matching changes. This one workflow was added with the user’s explicit ownership exception and remains scoped for Code Architect review. Existing Rust CI is unchanged; hosted Actions execution is not claimed as local evidence.
