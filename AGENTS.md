# Argos OSINT — Agent Orientation

Rust OSINT workbench (CLI + TUI) with Recon, Intel, Atlas and Brain pipelines, storing state in SurrealDB.

## Repo map

- `crates/argos-osint-core/` — OSINT, Recon, Brain, persistence. Storage: `src/storage/`. Storage contract tests: `tests/storage_*.rs` (shared harness `tests/support/`). Tool bindings: `src/recon/investigation/tool_io.rs`; provider adapters: `src/osint/providers.rs`.
- `crates/argos-osint-bin/` — CLI/TUI (`src/tui/`).
- `scripts/` — `agent_cargo.py` (isolated agent builds), `local_matrix.sh`, `test_surreal.py`, `release_check.mjs`, `tui_review.py`, `check_storage_deps.py`.
- `.github/workflows/` — CI, commit policy, Release Please. Owned by Code Architect. See [docs/ci.md](docs/ci.md).

## Where to look first

- [docs/README.md](docs/README.md) index; [architecture](docs/architecture.md), [concepts](docs/concepts.md), [providers](docs/providers.md), [usage](docs/usage.md), [conventions](docs/conventions.md), [versioning](docs/versioning.md), [diagrams](docs/diagrams.md).
- `.planning/<date-slug>/` — active designs; SurrealDB migration is `.planning/2026-10-22-surrealdb/` (design.md, acceptance.md).
- `.opencode/skills/` — `argos-plan`, `argos-implement`, `argos-commit`, `argos-tui-verify`, `ecc-*`, `graphify`. Tool allowlist: `.opencode/opencode.json`; see [opencode-v2.md](docs/opencode-v2.md).
- If `graphify-out/graph.json` exists, start repo questions with `graphify query` (then `graphify-out/wiki/index.md` or `GRAPH_REPORT.md`).
- TUI work: [tui-design-spec](docs/tui-design-spec.md), [tui-components](docs/tui-components.md), [tui-verification](docs/tui-verification.md) and the `argos-tui-verify` skill; Profile work also [profile-analytics-dashboard](docs/profile-analytics-dashboard.md). Capture with `scripts/tui_review.py`, scoped to affected views/sizes, and actually inspect the PNGs.

## Build and test

Rust 1.95.0 pinned; `.cargo/config.toml` uses the vendored `tools/protoc`. Agents always build through `python3 scripts/agent_cargo.py … --locked` (artifacts in `target/agents`; `target/debug` is the user's), with `ARGOS_EMBED` unset and never `--features lancedb`. `cargo fmt` runs directly.

```sh
scripts/local_matrix.sh --storage embedded   # fmt → clippy -D warnings → cargo test --workspace → test_surreal.py --mode embedded
```

That is the only leg agents run locally; the `server` leg runs in CI only ([docs/ci.md](docs/ci.md)). Serialize Cargo jobs sharing `target/agents`; never delete locks or kill unrelated Cargo processes (diagnose with `lsof -nP target/debug/.cargo-lock`). MiniLM tests are `#[ignore]` and need `ARGOS_EMBED=1`.

## Storage facts

- Default features `embedded,nlp`: embedded SurrealDB (RocksDB, in-process) under `<ARGOS_HOME>/storage/surreal`; `ARGOS_HOME` defaults to `~/.argos`.
- Server mode is opt-in: `--no-default-features --features server,nlp`, selected by `argos storage init --mode server` or `ARGOS_STORAGE_MODE=server|embedded` (env → `storage/config.json` → embedded).
- SQLite and LanceDB are being removed; don't add new code on them. `scripts/check_storage_deps.py` will guard their absence.
- Storage tests pick their mode from `ARGOS_SURREAL_MODE` and skip without it (and without `ARGOS_SURREAL_BIN`); run them via `test_surreal.py`.

## Workflow rules

- **Worktrees are edit-only:** in a feature worktree run only `cargo fmt`. One combined integration tree runs the gate (`local_matrix.sh --storage embedded`).
- **Repeat cap:** run the full gate 1–2 times, max 3. On failure fix with `cargo check --workspace --all-targets` and rerun only the failing `--test <name>`, then one full run.
- **Commits:** Conventional Commits (`feat` minor, `fix`/`perf` patch; breaking = `!` + `BREAKING CHANGE:` footer). Check with `node scripts/release_check.mjs message --file <path>` and `… versions`. Release Please owns version bumps; never hand-bump or invent `Release-As`. Stage exact paths; preserve unrelated changes.
- **`.github`** changes go through Code Architect.
- **Planning:** multi-phase work uses `.planning/<YYYY-MM-DD-slug>/` (`task_plan.md`, `findings.md`, `progress.md`); see [planning-artifacts](docs/planning-artifacts.md).
- **Scratch** lives only in `.agent-scratch/`, never in `crates/`, `docs/`, `scripts/`; never stage it.
- **Docs:** architecture/contract changes update docs and diagrams in the same change ([conventions](docs/conventions.md#architecture-and-concepts)). Tests stay offline.
