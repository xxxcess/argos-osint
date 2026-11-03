# Async Audit & Remediation — Team Handoff

## 1. Completed Scope (Committed Baseline)

### Milestone 1: Core Tasks, Jobs, Schedulers & Concurrency Throttling (`COMPLETE` & Verified)
- **Persistent Recon `ToolRunner` & Concurrency Cap**: Retained persistent `ToolRunner` on `recon::Service` (`crates/argos-osint-core/src/recon.rs`, `crates/argos-osint-core/src/recon/tool_runner.rs`) with `Semaphore(4)` throttling and in-flight deduplication.
- **Event-Driven Cancellation & RAII Cleanup**: Replaced polling loops with `CancellationToken` notifications (`CancellationNotifierGuard` drop-safe unregistration) across `recon.rs`, `tool_runner.rs`, `osint/holehe/mod.rs`, `job_registry.rs`, `atlas.rs`, and `tui/app.rs`.
- **Lease Renewal Heartbeat Guards**: Added background lease renewal tasks with immediate `.abort()` on stage completion in `storage/summary_flush.rs`, `storage/brain_scope.rs`, `storage/brief_stages.rs`, and `storage/neighborhood_brief.rs`.
- **Bounded Telemetry Queue**: Replaced unbounded queue in `storage/runtime.rs` with bounded `mpsc::channel` (4,096 capacity) and non-blocking drop telemetry.
- **WorkerPool & Blocking Offload**: Retained join handles in `scheduler.rs` (`WorkerPool::drop` joins worker threads) and offloaded synchronous filesystem/JSON operations in `osint/dataset.rs`, `osint/dork_generator.rs`, and `osint/whatsmyname.rs` via `tokio::task::spawn_blocking`.
- **Stress & Contract Tests**: Added `crates/argos-osint-core/tests/adversarial_recon_m1_stress.rs`, `crates/argos-osint-core/tests/storage_m1_challenger2_stress.rs`, and `crates/argos-osint-core/tests/storage_runtime.rs`.

### Milestone 2: Core Processes, I/O Pipes, Runtime Shutdown & Logging (`IMPLEMENTED` — Ready for Verification Gate)
- **Concurrent Pipe I/O**: Fixed sequential `stdin`/`stdout` pipe deadlock in `crates/argos-osint-core/src/subscription.rs` (`complete()`) via concurrent `tokio::select!` streaming with broken-pipe tolerance.
- **Non-Blocking Process Status & Signal Escalation**: Replaced synchronous `ps`/`kill` subprocess spawns in `crates/argos-osint-core/src/storage/server.rs` with non-blocking `libc::kill(pid, 0)` checks and `SIGTERM` $\to$ `SIGKILL` escalation; added bounded wait + `SIGKILL` escalation in `crates/argos-osint-bin/src/cli.rs`.
- **Filesystem Offload**: Offloaded synchronous archive extraction in `crates/argos-osint-core/src/storage/install.rs` and traffic log file appends in `crates/argos-osint-bin/src/stack/traffic.rs` via `spawn_blocking`.
- **Bounded Shutdown Drain**: Added 5-second timeout around `storage::runtime::drain()` in `crates/argos-osint-bin/src/main.rs`.
- **Non-Blocking TUI Runner**: Isolated interactive terminal event loop in `crates/argos-osint-bin/src/tui/app.rs`, `cli.rs`, `stack/mod.rs`, and `tui/topology.rs` onto a dedicated OS thread (`std::thread::spawn` + `Handle::block_on`) so Tokio worker threads are never pinned.
- **Tracing Subscriber Registration**: Added `crates/argos-osint-bin/src/logging.rs` (`TuiAwareWriter` suppressing stderr while raw TUI mode is active) and registered `tracing_subscriber` in both `argos-osint-bin/src/main.rs` and `argos-api-host/src/main.rs`.

---

## 2. Remaining Scope & Archetype Responsibilities

Follow `docs/wiki/code/opencode-v2.md` and `.opencode/AGENTS.md` conventions. Refer to agents strictly by role archetype (`coordinator`, `orchestrator`, `architect`, `coder`, `reviewer`, `tester`, `auditor`, `scribe`). Store all scratch artifacts/worktrees under `.agent-scratch/` and durable planning updates under `.planning/2026-10-27-async-execution-audit/`.

### Role Archetype Matrix

| Archetype | Responsibilities in Remaining Phases |
| --- | --- |
| **`coordinator`** (Team Coordinator) | Entry-point liaison: confirm scope/exclusions (or fast-bypass with `[skip-interview]`), dispatch `orchestrator` for Milestones 2–5, and report verified completion. |
| **`orchestrator`** (Project Orchestrator) | Drive Milestone 2 gate sign-off and Milestones 3–5 execution, manage `.planning/2026-10-27-async-execution-audit/`, and enforce the 3-way verification gate (`reviewer` + `tester` + `auditor`). |
| **`architect`** (Architecture & Spec Specialist) | Provide `AUDIT_UNIT` / `CONSULT` specifications for Milestone 3 (Web Client & API Server) and advise on cross-crate or UI contracts. |
| **`coder`** (Implementation & Worktree Admin) | Implement Milestone 3 fixes in isolated worktrees (`.agent-scratch/worktrees/`) or `INTEGRATION_ROOT` via `EDIT_UNIT` workers, and remediate any findings surfaced by the gate. |
| **`reviewer`** (Code Quality & Style Reviewer) | Audit Milestone 2 and Milestone 3 diffs against `argos-rust-style`, `argos-react-ui`, `argos-react-patterns`, and concurrency/async contracts (`APPROVE \| REJECT`). |
| **`tester`** (Empirical Verification Gate) | Execute host-native Rust checks/tests (embedded SurrealDB mode) for Milestone 2 and full E2E Rust + Web verification gates for Milestone 4 (`PASS \| FAIL \| BLOCKED \| PENDING`). |
| **`auditor`** (Forensic Integrity Auditor) | Inspect git diffs, tests, and evidence for Milestones 2–4 to certify zero facade code, zero test weakening, and zero workspace pollution (`CLEAN \| INTEGRITY_VIOLATION`). |
| **`scribe`** (Documentation & Wiki Maintainer) | Execute Milestone 5 (`docs/` and `docs/wiki/` updates, tag registry refresh, and `.planning/2026-10-27-async-execution-audit/` checklist completion). |

---

## 3. Step-by-Step Execution Plan for Next Team

### Step 1: Milestone 2 Verification Gate (`reviewer` + `tester` + `auditor`)
1. **`tester`**: Run targeted verification on Milestone 2 crates in `INTEGRATION_ROOT`:
   ```bash
   python3 scripts/agent_cargo.py clippy --locked --workspace --all-targets -- -D warnings
   python3 scripts/agent_cargo.py test --locked -p argos-osint-core --lib
   python3 scripts/agent_cargo.py test --locked -p argos-osint-bin --lib
   python3 scripts/agent_cargo.py test --locked -p argos-osint-bin --test stack_cli
   python3 scripts/agent_cargo.py test --locked -p argos-api-host
   ```
2. **`reviewer` & `auditor`**: Audit Milestone 2 changes (`subscription.rs`, `storage/server.rs`, `storage/install.rs`, `cli.rs`, `main.rs`, `logging.rs`, `tui/app.rs`, `stack/traffic.rs`) for `argos-rust-style` compliance, process/signal safety, and forensic integrity (`CLEAN`).

### Step 2: Milestone 3 Implementation — Web Client Signals, Timeouts & Error Boundaries (`architect` + `coder` + `reviewer` + `auditor`)
> **Note**: `apps/web-desktop` is a static export (`output: "export"`). Do not add Next.js server routes or actions.

1. **`coder`** — Edit `packages/api-client/src/index.tsx`:
   - **AbortSignal Lifecycle**: Keep `combined.cleanup?.()` in a `try ... finally` block that wraps **both** `await fetch(...)` and `await response.text()` so aborting a query while streaming the response body cancels the body read.
   - **Mutation AbortSignal**: Allow `useAction` mutations to accept/forward an optional `AbortSignal` to `request<T>`.
   - **Timeout Alignment**: Allow `request<T>` / `combineSignals` to accept a configurable timeout (default `10_000` ms, extensible to `30_000` ms for heavy topology/brief endpoints to match the 30s Axum timeout in `crates/argos-osint-api/src/lib.rs`).
2. **`coder`** — Add React `ErrorBoundary` (`packages/ui/src/` & `packages/intel/src/topology/Topology.tsx`):
   - Load skills `argos-react-ui` and `argos-react-patterns`.
   - Implement a terminal-desk themed `ErrorBoundary` in `@argos/ui` (or `packages/intel`) with a clean retry action using shared primitives.
   - Wrap the lazy `<Suspense>` block around `ForceCanvas` in `packages/intel/src/topology/Topology.tsx` (and top-level `Workspace` if appropriate) so chunk load or canvas failures do not white-screen the app.
3. **`coder`** — Disconnect Telemetry in `scripts/serve_web.mjs`:
   - Track whether `finish` fired; attach `res.once("close", ...)` to log aborted client requests when `!res.writableEnded`.
4. **`reviewer` & `auditor`**: Review all Milestone 3 diffs for React Compiler compatibility, `argos-react-ui` look-and-feel compliance, signal cleanup safety, and forensic authenticity.

### Step 3: Milestone 4 — Full Host-Native E2E Verification (`tester` + `auditor`)
1. **`tester`** — Run Rust embedded verification:
   ```bash
   ARGOS_SURREAL_MODE=embedded ARGOS_SURREAL_REQUIRED=1 \
     python3 scripts/agent_cargo.py test --locked -p argos-osint-core --test storage_runtime --test storage_m1_challenger2_stress
   python3 scripts/agent_cargo.py test --locked -p argos-osint-core --test adversarial_recon_m1_stress
   python3 scripts/agent_cargo.py test --locked -p argos-osint-api --test api_contract
   ```
2. **`tester`** — Run Web verification (using pinned Node 24.22.1 if present at `~/.nvm/versions/node/v24.22.1/bin`):
   ```bash
   pnpm typecheck
   pnpm lint
   node scripts/check_web_build.mjs
   pnpm test:e2e
   ```
3. **`tester`** — Run full local embedded gate (1–2 runs max):
   ```bash
   scripts/local_matrix.sh --storage embedded
   ```

### Step 4: Milestone 5 — Documentation & Wiki Refresh (`scribe` + `orchestrator`)
1. **`scribe`**: Verify and update documentation to reflect all M1–M3 contracts:
   - `docs/wiki/code/architecture.md`: Document persistent `ToolRunner` semaphore(4), `CancellationNotifierGuard` RAII pattern, bounded `storage::runtime` channel (4,096), `TuiAwareWriter` tracing subscriber, and dedicated OS thread for `tui::run`.
   - `docs/wiki/code/conventions.md`: Document `spawn_blocking` offload requirements, concurrent child pipe I/O (`tokio::select!`), non-blocking `libc::kill(pid, 0)` probing, and `SIGTERM` $\to$ `SIGKILL` escalation.
   - `docs/wiki/code/intel-web.md`: Document full-body `AbortSignal` lifecycle, configurable 10s/30s client timeouts, `ErrorBoundary` wrapping around lazy topology canvas, and `serve_web.mjs` disconnect logging.
   - `docs/wiki/tags.md`: Verify `@tag:async-runtime`, `@tag:process-lifecycle`, `@tag:api-timeouts`, and `@tag:abort-signal` cross-references across `docs/` and `crates/`.
2. **`scribe` & `orchestrator`**: Mark all phases complete in `.planning/2026-10-27-async-execution-audit/task_plan.md` and `progress.md`, prune `.agent-scratch/` temporary files/worktrees, and commit via `coder` with a Conventional Commit message validated by `node scripts/release_check.mjs message --file <path>`.
