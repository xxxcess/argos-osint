# Conventions

Rules that keep Argos and agent work aligned. Product behavior lives in [architecture.md](architecture.md). Agent commands live in [AGENTS.md](../AGENTS.md).

## Checks

Order matches CI: `cargo fmt --all --check` → `python3 scripts/agent_cargo.py clippy --workspace --all-targets --locked --no-default-features -- -D warnings` → `python3 scripts/agent_cargo.py test --workspace --locked --no-default-features` (offline, `ARGOS_EMBED` unset, no LanceDB). Local default builds include the `lancedb` feature. MiniLM tests are `#[ignore]` and need `ARGOS_EMBED=1`.

Agent compilation uses `scripts/agent_cargo.py` and `target/agents`; user runs keep `target/debug`. Screenshot capture uses the same isolated cache. See [agent Cargo isolation](../AGENTS.md#agent-cargo-isolation) for ownership and lock diagnosis.

## TUI

All agents follow [tui-verification.md](tui-verification.md) and the repo [argos-tui-verify skill](../.opencode/skills/argos-tui-verify/SKILL.md). Use `scripts/tui_review.py` for fresh fixture galleries. Inspect PNGs, action tests and metric tests separately; capture is not visual approval. [Session evidence](tui-verification-session-2026-10-10.md) records the defects behind this convention. Keep the viewport/state matrix proportional to affected presets; Profile uses 160×50, 120×40, 100×32, 100×24, 80×24, 60×18 and 40×20. Artifacts are fixture cells/PNG/manifest/review ledger under a named plan; avoid live secrets.

- Layout, hit-test, and Tab order share rectangles. `LayoutRegistry` fills during `draw`. Focus order uses registered targets from the active layout scope. Figure: [tui-shell.html](diagrams/tui-shell.html).
- Internal IDs stay `Osint` / `Providers` / `System` even when the labels are Tools / Models / Profile.
- Empty focused fields keep their placeholder until the user types. Placeholders are never saved. Keys stay masked.
- Long panes own an offset, a wrapped extent, and a `see more` footer. Offsets are not `u16`-truncated page math.
- Investigation panel width is `floor(0.30 × terminal viewport)` when the screen is at least 110 columns. Full-report cards are `floor(0.70 × viewport)` wide.

## Schema

- Bump `SCHEMA_VERSION` and add an additive `user_version` step in `store.rs`.
- `CREATE TABLE IF NOT EXISTS` for any table that landed in a SQL file after that schema version already shipped. Existing databases skip the original `include_str!` apply. Figure: [schema-core.html](diagrams/schema-core.html).
- Create a table before any `UPDATE`/`SELECT` that reads it (v23 used to query `intel_report_attempts` before v24 created it).
- Do not delete legacy secrets when saving a new provider account.

## Tests

- Offline by default. HTTP fixtures and fake executors; no live keys in the suite.
- Coverage tests in core: every tool input has a binding kind, extractors, and producers.
- TUI: `ratatui::backend::TestBackend` at 160×50, 120×40, 100×36, 80×24 as needed. `click` draws first.

## Agent scratch

Scratch scripts and logs go only under `.agent-scratch/` (gitignored). Never write `patch_*.py` at the repo root or in `crates/`. After the check gate passes, delete that task's scratch files.

## Planning files

Multi-step work uses **planning-with-files**: `task_plan.md`, `findings.md`, `progress.md` under `.planning/<YYYY-MM-DD-slug>/`.

- Root copies of those names are gitignored leftovers
- One orchestrator owns `task_plan.md`
- Untrusted web text goes in `findings.md` only

## Graphify

Query `graphify-out/graph.json` before broad exploration. Expand query tokens from graph vocabulary. After code changes: `graphify update .`. Dirty graph files are expected.

## Diagrams

New figures follow [diagrams.md](diagrams.md): self-contained HTML + SVG, no Mermaid, no shadows, accent on at most two nodes. Link each figure from this index and the matching Markdown doc.

TUI changes follow [the design contract](tui-design-spec.md) and [component catalog](tui-components.md). Name the components/preset before editing; preserve metric semantics, widget IDs and palette. Profile uses registered geometry and independent grid/report scrolling.
