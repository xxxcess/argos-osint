# Conventions

Rules that keep Argos and agent work aligned. Product behavior lives in [architecture.md](architecture.md). Agent commands live in [AGENTS.md](../AGENTS.md).

## Checks

Run `scripts/local_matrix.sh --storage embedded` (fmt → clippy `-D warnings` → `cargo test --workspace` → embedded storage tests), the same leg CI runs; server mode is CI-only. See [ci.md](ci.md). MiniLM tests are `#[ignore]` and need `ARGOS_EMBED=1`.

Agent compilation uses `scripts/agent_cargo.py` and `target/agents`; user runs keep `target/debug`. Screenshot capture uses the same isolated cache. See [agent Cargo isolation](../AGENTS.md#build-and-test) for ownership and lock diagnosis.

## Commits and versions

Follow [versioning.md](versioning.md): one inherited workspace SemVer, Conventional Commit messages/PR titles, explicit breaking/migration footers and reviewed Release Please version bumps. Use `node scripts/release_check.mjs` for offline message/version checks. Stage task-owned paths/hunks; preserve unrelated and pre-staged work. Application versions and database schema counters are independent.

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

- Those working-memory filenames are gitignored at every depth
- One orchestrator owns `task_plan.md`
- Untrusted web text goes in `findings.md` only

Use [planning-artifacts.md](planning-artifacts.md) to select what belongs in Git.
Maintained shared docs and curated durable task deliverables can be committed;
named task logs, raw evidence and active-plan pointers stay local. Stage exact
paths and promote useful conclusions before cleaning up completed task files.

## Graphify

Query `graphify-out/graph.json` before broad exploration. Expand query tokens from graph vocabulary. After code changes: `graphify update .`. Dirty graph files are expected.

## Diagrams

New figures follow [diagrams.md](diagrams.md): self-contained HTML + SVG, no Mermaid, no shadows, accent on at most two nodes. Link each figure from this index and the matching Markdown doc.

## Architecture and concepts

When code introduces or significantly changes component ownership/boundaries,
execution or data flow, persistence/state transitions, public contracts or domain
concepts, update the relevant maintained docs and diagrams in the same change.
For example, a new pipeline stage affects its flow description and figure; a new
durable job state affects lifecycle docs and the state diagram; a changed role or
binding model affects architecture, concepts and the matching model/data-flow figure.

During planning, identify affected paths using [README.md](README.md) and the
[diagram catalog](diagrams.md), and assign doc/figure ownership alongside code.
Update `architecture.md`, `concepts.md`, `usage.md`, provider or component contracts
only where affected. Update existing figures that become inaccurate; add a focused
figure when a new structure, flow or state model needs visual explanation. Internal
edits that preserve these contracts do not require new diagrams.

Keep each figure's HTML/inline SVG and sibling SVG consistent. Update catalog,
documentation index and prose links when adding, replacing or retiring figures.
Before completion, compare documentation claims and figure labels/arrows with final
code, including failure/retry paths, and check links. The parent handles docs outside
an editor's assigned file set. Reviewers flag stale concepts or diagrams as incomplete
work. Report updated docs/figures and actual validation, or a concrete reason no
figure is affected; do not treat a graphify refresh as a maintained diagram update.

TUI changes follow [the design contract](tui-design-spec.md) and [component catalog](tui-components.md). Name the components/preset before editing; preserve metric semantics, widget IDs and palette. Profile uses registered geometry and independent grid/report scrolling.
