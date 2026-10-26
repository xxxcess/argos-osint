# Contributor workflow

[Code route](index.md) · [Wiki](../../index.md)

Detailed agent rules, loaded before editing or running checks.

## Build and test

Rust 1.95.0 pinned. Agents always build through `python3 scripts/agent_cargo.py … --locked` (artifacts in `target/agents`; `target/debug` is the user's), with `ARGOS_EMBED` unset. `cargo fmt` runs directly.

```sh
scripts/local_matrix.sh --storage embedded   # fmt → clippy -D warnings → cargo test --workspace → test_surreal.py --mode embedded
```

That is the only leg agents run locally; the `server` leg runs in CI only ([docs/wiki/code/ci.md](ci.md)). Serialize Cargo jobs sharing `target/agents`; never delete locks or kill unrelated Cargo processes (diagnose with `lsof -nP target/debug/.cargo-lock`). MiniLM tests are `#[ignore]` and need `ARGOS_EMBED=1`.

## Storage facts

- Default features `embedded,nlp`: embedded SurrealDB (RocksDB, in-process) under `<ARGOS_HOME>/storage/surreal`; `ARGOS_HOME` defaults to `~/.argos`.
- Server mode is opt-in: `--no-default-features --features server,nlp`, selected by `argos storage init --mode server` or `ARGOS_STORAGE_MODE=server|embedded` (env → `storage/config.json` → embedded).
- Storage is SurrealDB only. CI fails if `rusqlite` or `lancedb` appears in `cargo tree`.
- Storage tests pick their mode from `ARGOS_SURREAL_MODE` and skip without it (and without `ARGOS_SURREAL_BIN`); run them via `test_surreal.py`.

## Workflow rules

- **Worktrees are edit-only:** in a feature worktree run only `cargo fmt`. One combined integration tree runs the gate (`local_matrix.sh --storage embedded`).
- **Repeat cap:** run the full gate 1–2 times, max 3. On failure fix with `cargo check --workspace --all-targets` and rerun only the failing `--test <name>`, then one full run.
- **Commits:** Conventional Commits (`feat` minor, `fix`/`perf` patch; breaking = `!` + `BREAKING CHANGE:` footer). Check with `node scripts/release_check.mjs message --file <path>` and `… versions`. Release Please owns version bumps; never hand-bump or invent `Release-As`. Stage exact paths; preserve unrelated changes.
- **`.github`** changes go through Code Architect.
- **Planning:** multi-phase work uses `.planning/<YYYY-MM-DD-slug>/` (`task_plan.md`, `findings.md`, `progress.md`); see [planning-artifacts](planning-artifacts.md).
- **Scratch** lives only in `.agent-scratch/`, never in `crates/`, `docs/`, `scripts/`; never stage it.
- **Docs:** architecture/contract changes update docs and diagrams in the same change ([conventions](conventions.md#architecture-and-concepts)). Tests stay offline.
