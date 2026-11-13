# Conventions

[Code route](index.md) · [Wiki](../../index.md)

Conventions for Fleetworks contributors. Product behavior lives in [architecture.md](architecture.md). Agent navigation starts at [AGENTS.md](../../../AGENTS.md); detailed commands live in [contributing.md](contributing.md).

## Checks and delivery

Use [contributor workflow](contributing.md) for builds, isolated artifacts, worktrees, retries and commits; [versioning](versioning.md) defines release policy. Tests remain offline.

## Historical terminal reference

The terminal executables and their capture helpers are retired. Their [design](tui-design-spec.md), [components](tui-components.md), [Profile metrics](profile-analytics-dashboard.md) and [verification records](tui-verification.md) remain historical references. Use [native verification](native-verification.md) and React guidance for supported clients. [Retirement disposition](../../archive/cli-retirement.md)

## Schema

Domain storage is SurrealDB; the Jobs service owns SQLite operational logs. Read [bootstrap.rs](../../../crates/argos-osint-core/src/storage/bootstrap.rs), the affected module schemas and [storage contracts](storage.md) before changing persistence. Bootstrap applies schemas in dependency order and guards the stored version; do not copy legacy SQLite `user_version` migration instructions. Document compatibility/migration decisions and verify them through the storage harness. Never delete legacy secrets when saving a new provider account.

## Tests

- Offline by default. HTTP fixtures and fake executors; no live keys in the suite.
- Coverage tests in core: every tool input has a binding kind, extractors, and producers.
- Native: use actual Rune production views and Runtime handlers, with selected PNG/tree inspection; the retired terminal TestBackend records are historical.

| Test boundary | Fixture practice |
| --- | --- |
| Registered authorization | Exercise real registration/admission in integration tests; use forged claims to verify rejection. |
| Process environment | Share one guard across consumers and restore previous values. |
| Runtime budgets | Reuse the existing shared test gate; a module-local mutex cannot coordinate other consumers. |
| Revisions | Compare durable values within the same root/context; keep process counters separate. |

## Planning and scratch

Use the [planning format and retention policy](planning-artifacts.md#working-format); the plan owner updates phase checkboxes and Next Step as work completes. [Contributor workflow](contributing.md#workflow-rules) owns scratch and staging rules.

## Wiki navigation and maintenance

Start at [docs/index.md](../../index.md), follow the concept/code routes and [searchable tags](../tags.md), then verify against source and tests. Follow [setup/refresh wiki](../maintenance.md) when structure or contracts change. Maintain links, tag pairs and diagrams in the same change.

## Diagrams

New figures follow [diagrams.md](../../diagrams.md): self-contained HTML + SVG, no Mermaid, no shadows, accent on at most two nodes. Link each figure from the diagram catalog and the matching wiki page.

## Architecture and concepts

Update affected canonical docs, contracts, tags and diagrams when behavior, ownership, persistence or public interfaces change. Plan that scope with the implementation, review it before freeze, and repeat only for a changed relevant delta; see [finish-line policy](agent-workflow.md#scope-and-finish-line).

Keep paired HTML/SVG labels and arrows consistent with final source; update indexes only for added, moved or retired pages. Internal edits that preserve contracts need no new diagram. Check links and inspect changed figures. Do not turn an implementation task into a general wiki refresh.

TUI and React changes follow their linked design/component/theme guidance; preserve metric semantics, widget identities and layout/interaction contracts.

## Rust style

Rust edits and reviews load the [argos-rust-style skill](../../../.opencode/skills/argos-rust-style/SKILL.md): Google's Rust Style Guide for imports, errors, types, traits, naming, docs, and implementation. `cargo fmt` formats. Library rules apply to `argos-osint-core` and `argos-osint-api`; application rules apply to `fleetworks-desktop`, `fleetworks-search-index` and `argos-api-host`. The async rules below and the current crate layout override that guide's Chromium and Fuchsia notes.

## Async runtime and process safety

Rules for asynchronous execution and process management:

- **Non-blocking worker threads**: Never invoke synchronous sleeps (`std::thread::sleep`), blocking disk I/O, or CPU-bound loops on Tokio runtime threads. Use `tokio::task::spawn_blocking` to offload synchronous work (such as hardware metric sampling or synchronous NLP parsing) or native `tokio::fs` and `tokio::net` primitives.
- **Large async callbacks**: Box large operations at their actual async boundary, including worker callbacks. For task-local scopes, box the operation before constructing the scope. Boxing an outer async wrapper can leave the callback inline in its poll frame. Diagnose stack overflows with native traces and measured frame sizes; fix the allocation boundary and rerun the real public path on the normal stack instead of increasing test stacks.
- **Synchronous drop safety**: Never spawn detached async tasks (`tokio::spawn`) within `Drop::drop` to perform state cleanup. State maps requiring teardown on drop must use synchronous synchronization (`std::sync::Mutex`) so entries are cleared synchronously and deterministically.
- **RAII task cancellation**: Any background loop or lease renewal task spawned during stage execution must be held by an RAII guard that stores an atomic cancellation flag and invokes `handle.abort()` on drop. When exiting normally, the guard's handle is taken and cleanly awaited.
- **Subprocess ownership**: Always set `.kill_on_drop(true)` on `tokio::process::Command` when spawning external processes, guaranteeing that children are terminated if the supervising future or task is cancelled or dropped.
- **Dual-signal shutdown**: Supervisors and long-running daemons must handle both `SIGINT` and `SIGTERM` gracefully on Unix, forwarding signals to child processes and completing orderly teardown before exit.
- **Process table hygiene**: Detached external command invocations (e.g. system browser openers) must explicitly wait on child process handles in dedicated background threads to harvest exit status codes and prevent zombie (`<defunct>`) processes.
- **spawn_blocking offload**: Blocking or synchronous work must stay off Tokio worker threads. Use bounded `tokio::task::spawn_blocking` or dedicated owned workers at actual service boundaries; native clients keep engine execution on the backend.
- **Concurrent child pipe I/O**: `crates/argos-osint-core/src/subscription.rs::complete` writes the prompt to the child's stdin and reads stdout JSONL lines concurrently in a `tokio::select!` loop. A broken pipe on the stdin write (`ErrorKind::BrokenPipe`, child closed stdin early) is tolerated: stdin is dropped and the stdout reader continues to drain the child's output instead of failing the whole completion.
- **Non-blocking process liveness probing**: `crates/argos-osint-core/src/storage/server.rs::pid_alive` probes a process with non-blocking `libc::kill(pid, 0)` (signal 0, no signal delivered). `EPERM` means the process exists but is owned by another user (treated as alive); `ESRCH` means it does not exist. Non-positive or overflowing PIDs are strictly rejected.
- **Bounded SIGTERM→SIGKILL escalation**: `storage/server.rs::stop_server_process` sends SIGTERM, waits an 8-second grace, then escalates to SIGKILL with a further 5-second kill grace, bailing if the server still does not stop.
