# Conventions

[Code route](index.md) · [Wiki](../../index.md)

Rules that keep Argos and agent work aligned. Product behavior lives in [architecture.md](architecture.md). Agent navigation starts at [AGENTS.md](../../../AGENTS.md); detailed commands live in [contributing.md](contributing.md).

## Checks and delivery

Use [contributor workflow](contributing.md) for builds, isolated artifacts, worktrees, retries and commits; [versioning](versioning.md) defines release policy. Tests remain offline.

## TUI

All agents follow [tui-verification.md](tui-verification.md) and the repo [argos-tui-verify skill](../../../.opencode/skills/argos-tui-verify/SKILL.md). Use `scripts/tui_review.py` for fresh fixture galleries. Inspect PNGs, action tests and metric tests separately; capture is not visual approval. [Session evidence](../../archive/tui-verification-session-2026-10-10.md) records the defects behind this convention. Keep the viewport/state matrix proportional to affected presets; Profile uses 160×50, 120×40, 100×32, 100×24, 80×24, 60×18 and 40×20. Artifacts are fixture cells/PNG/manifest/review ledger under a named plan; avoid live secrets.

- Layout, hit-test, and Tab order share rectangles. `LayoutRegistry` fills during `draw`. Focus order uses registered targets from the active layout scope. Figure: [tui-shell.html](../../diagrams/tui-shell.html).
- Internal IDs stay `Osint` / `Providers` / `System` even when the labels are Tools / Models / Profile.
- Empty focused fields keep their placeholder until the user types. Placeholders are never saved. Keys stay masked.
- Long panes own an offset, a wrapped extent, and a `see more` footer. Offsets are not `u16`-truncated page math.
- Investigation panel width is `floor(0.30 × terminal viewport)` when the screen is at least 110 columns. Full-report cards are `floor(0.70 × viewport)` wide.

## Schema

Current storage is SurrealDB. Read [bootstrap.rs](../../../crates/argos-osint-core/src/storage/bootstrap.rs), the affected module schemas and [storage contracts](storage.md) before changing persistence. Bootstrap applies schemas in dependency order and guards the stored version; do not copy legacy SQLite `user_version` migration instructions. Document compatibility/migration decisions and verify them through the storage harness. Never delete legacy secrets when saving a new provider account.

## Tests

- Offline by default. HTTP fixtures and fake executors; no live keys in the suite.
- Coverage tests in core: every tool input has a binding kind, extractors, and producers.
- TUI: `ratatui::backend::TestBackend` at 160×50, 120×40, 100×36, 80×24 as needed. `click` draws first.

## Planning and scratch

Use the [planning format and retention policy](planning-artifacts.md#working-format); the plan owner updates phase checkboxes and Next Step as work completes. [Contributor workflow](contributing.md#workflow-rules) owns scratch and staging rules.

## Wiki navigation and maintenance

Start at [docs/index.md](../../index.md), follow the concept/code routes and [searchable tags](../tags.md), then verify against source and tests. Follow [setup/refresh wiki](../maintenance.md) when structure or contracts change. Maintain links, tag pairs and diagrams in the same change.

## Diagrams

New figures follow [diagrams.md](../../diagrams.md): self-contained HTML + SVG, no Mermaid, no shadows, accent on at most two nodes. Link each figure from the diagram catalog and the matching wiki page.

## Architecture and concepts

When code introduces or significantly changes component ownership/boundaries,
execution or data flow, persistence/state transitions, public contracts or domain
concepts, update the relevant maintained docs and diagrams in the same change.
For example, a new pipeline stage affects its flow description and figure; a new
durable job state affects lifecycle docs and the state diagram; a changed role or
binding model affects architecture, concepts and the matching model/data-flow figure.

During planning, identify affected paths using [the wiki](../../index.md) and the
[diagram catalog](../../diagrams.md), and assign doc/figure ownership alongside code.
Update [architecture](architecture.md), [glossary](../concepts/glossary.md), [usage](../concepts/usage.md), provider or component contracts
only where affected. Update existing figures that become inaccurate; add a focused
figure when a new structure, flow or state model needs visual explanation. Internal
edits that preserve these contracts do not require new diagrams.

Keep each figure's HTML/inline SVG and sibling SVG consistent. Update catalog,
documentation index and prose links when adding, replacing or retiring figures.
Before completion, compare documentation claims and figure labels/arrows with final
code, including failure/retry paths, and check links. The parent handles docs outside
an editor's assigned file set. Reviewers flag stale concepts or diagrams as incomplete
work. Report updated docs/figures and actual validation, or a concrete reason no
figure is affected; do not treat an index-only refresh as a maintained diagram update.

TUI changes follow [the design contract](tui-design-spec.md) and [component catalog](tui-components.md). Name the components/preset before editing; preserve metric semantics, widget IDs and palette. Profile uses registered geometry and independent grid/report scrolling. React Native UI follows the [Argos look and feel](react-ui.md#argos-look--feel-reference): name the screen type and density, use theme tokens, design components creatively inside the Argos theme, and render topology graphs with the force-graph canvas.

## Rust style

Rust edits and reviews load the [argos-rust-style skill](../../../.opencode/skills/argos-rust-style/SKILL.md): Google's Rust Style Guide for imports, errors, types, traits, naming, docs, and implementation. `cargo fmt` formats. Library rules apply to `argos-osint-core` and `argos-osint-api`; application rules apply to `argos-osint-bin` and `argos-api-host`. The async rules below and the current crate layout override that guide's Chromium and Fuchsia notes.

## Async runtime and process safety

Rules for asynchronous execution and process management:

- **Non-blocking worker threads**: Never invoke synchronous sleeps (`std::thread::sleep`), blocking disk I/O, or CPU-bound loops on Tokio runtime threads. Use `tokio::task::spawn_blocking` to offload synchronous work (such as hardware metric sampling or synchronous NLP parsing) or native `tokio::fs` and `tokio::net` primitives.
- **Synchronous drop safety**: Never spawn detached async tasks (`tokio::spawn`) within `Drop::drop` to perform state cleanup. State maps requiring teardown on drop must use synchronous synchronization (`std::sync::Mutex`) so entries are cleared synchronously and deterministically.
- **RAII task cancellation**: Any background loop or lease renewal task spawned during stage execution must be held by an RAII guard that stores an atomic cancellation flag and invokes `handle.abort()` on drop. When exiting normally, the guard's handle is taken and cleanly awaited.
- **Subprocess ownership**: Always set `.kill_on_drop(true)` on `tokio::process::Command` when spawning external processes, guaranteeing that children are terminated if the supervising future or task is cancelled or dropped.
- **Dual-signal shutdown**: Supervisors and long-running daemons must handle both `SIGINT` and `SIGTERM` gracefully on Unix, forwarding signals to child processes and completing orderly teardown before exit.
- **Process table hygiene**: Detached external command invocations (e.g. system browser openers) must explicitly wait on child process handles in dedicated background threads to harvest exit status codes and prevent zombie (`<defunct>`) processes.

