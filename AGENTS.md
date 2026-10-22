# Argos OSINT — Agent Instructions

## TUI work

For `crates/argos-osint-bin/src/tui/`, read [the design spec](docs/tui-design-spec.md), relevant [component docs](docs/tui-components.md), [verification guide](docs/tui-verification.md), and the repo [argos-tui-verify skill](.opencode/skills/argos-tui-verify/SKILL.md) (OpenCode: `argos-tui-verify` or `/tui-verify`). Use `scripts/tui_review.py` for fresh TestBackend captures, inspect PNGs with an image-capable viewer, and report visual, interaction, and metric results separately. Capture generation alone is not review; report unavailable references, image review, or live checks. See [session evidence](docs/tui-verification-session-2026-10-10.md).

For Profile work, also read [the Profile dashboard contract](docs/profile-analytics-dashboard.md). It defines 20 primary views (Intel 4, Recon 5, Atlas 3, Models 4, Tools 4); Summary reuses views. Before editing, name the components (AnalyticsCard, TimePlot, DetailTable, Meter, ScrollPane) and Dashboard/Report preset. Extend shared `profile_components` when needed; `profile_layout::LayoutResult` owns geometry used by drawing, focus, and mouse targets. Preserve theme RGB/tokens, primary IDs, metric semantics/history, System, and Configs. Verify affected views and states; when shared Profile geometry changes, capture 160×50, 120×40, 100×32, 80×24, 60×18, and too-small sizes. For the navigation/component overhaul, also preserve three Profile pages, independent TabBar focus/activation, nested dataset offsets, background image resources, exact-revision Verify/Save, and `usize` transcript scroll. Scope captures to affected apps/pages/states; select `--test` and `--include`, use `--size` for affected thresholds, and record `--reason`. Shared components need representative consumers; shared shell/theme/global navigation or a full overhaul needs both fixture suites with `--all`. Unrelated apps need no captures. Child prompts must include components, preset, owned files, read-only references, and acceptance checks.

## Workspace and commands

Rust workspace: `argos-osint-core` (OSINT, Recon, Brain, persistence) and `argos-osint-bin` (CLI/TUI). See [docs/README.md](docs/README.md), [architecture](docs/architecture.md), and [diagrams](docs/diagrams.md).

```sh
cargo run -p argos-osint-bin
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace                 # offline; ARGOS_EMBED unset
ARGOS_EMBED=1 cargo test -p argos-osint-core -- --ignored minilm
```

Check order is fmt → clippy → test. MiniLM tests download a ~23 MB model. Rust 1.95.0 is pinned (minimum 1.95, required by the SurrealDB 3.3.4 SDK); `.cargo/config.toml` uses the vendored `tools/protoc` shim. First default build is slow and needs a C toolchain; local default builds include LanceDB.

## OpenCode and Cargo

OpenCode agents build/test without LanceDB: every `build`, `test`, or `clippy` uses `python3 scripts/agent_cargo.py`, `--locked --no-default-features`, and unset `ARGOS_EMBED`; never add `--features lancedb`. The wrapper isolates artifacts in `target/agents`; reserve `target/debug` for the user's `cargo run`. Serialize jobs sharing that cache, keep it, and never delete locks or kill unrelated Cargo processes. For artifact-lock issues, identify the owner with `lsof -nP target/debug/.cargo-lock`; registry locks are separate. `cargo fmt` can run directly.

OpenCode check sequence (shell timeout 600000):

```sh
cargo fmt --all --check
python3 scripts/agent_cargo.py clippy --workspace --all-targets --locked --no-default-features -- -D warnings
python3 scripts/agent_cargo.py test --workspace --locked --no-default-features
```

After a successful trio, run `graphify update .`. On `build`, load `argos-implement` and delegate each disjoint file unit for the current phase to `ecc-edit` (same turn in background if multiple); do not edit those files or poll while children run. Editors format changed Rust files and do not test. On failure, route relevant output back to the owning editor, rerun the failed command, then the trio. `plan` does not launch editors. OpenCode V2 tool rules: use the provider schema; never call `execute` or invent namespaces. The primary allowlist is `.opencode/opencode.json`; tools are read, grep, glob, edit, write, shell, skill, webfetch, websearch, question, and subagent. `build` may use explore/planner/reviewer/edit; `plan` may use explore/planner/reviewer. Repo questions start with `graphify query` when `graphify-out/graph.json` exists; expand vocabulary from `graphify-out/.vocab.txt`, then use `path`/`explain` and read source locations. Prefer `graphify-out/wiki/index.md` when present; use `graphify-out/GRAPH_REPORT.md` for architecture review or when graph queries are thin. Dirty graph output after hooks is expected; skip graphify only if the graph itself is the bug or the user asks.

## Planning and scratch

For 5+ tool calls, multi-phase work, or work that must survive compaction, use planning-with-files (`~/.agents/skills/planning-with-files/SKILL.md`); OpenCode uses `argos-plan`. Name plans `.planning/<YYYY-MM-DD-slug>/`, pin `PLAN_ID` if needed, and maintain `task_plan.md`, `findings.md`, and `progress.md` (one orchestrator owns the plan). Graphify first, then plan from the subgraph. Follow [planning-artifacts guidance](docs/planning-artifacts.md): keep working memory and raw evidence local; stage only eligible durable deliverables by exact path and promote lasting requirements to `docs/`. Do not paste the plan into chat.

Agent-only scripts, logs, and temporary Rust files belong only in `.agent-scratch/`; never beside Cargo files or in `crates/`, `docs/`, or `scripts/`, and never stage them. Prefer editor changes; reread before any line-number rewrite. After fmt/clippy/test, remove task scratch; if work is incomplete, leave and list it in the handoff. Keep `scripts/` for documented project utilities. Review artifacts belong in the named plan; selected durable fixture PNGs may go in `docs/screenshots/`.

## Commits, PRs, and releases

Follow [versioning.md](docs/versioning.md). Use Conventional Commits: `feat` is minor, `fix`/`perf` patch; maintenance types do not release. Breaking changes use `!` plus a `BREAKING CHANGE:` footer describing impact/migration (minor in 0.x, major from 1.0). Assess CLI/output, core API, config/data compatibility, and documented workflows. Release Please owns synchronized workspace, lockfile, `version.txt`, manifest, and changelog bumps via reviewed PRs/tags; never bump ordinary commits or invent `Release-As`.

Preserve unrelated and pre-staged changes. Stage exact task paths/hunks and inspect the full staged diff. Before committing, use a scratch message file and run `node scripts/release_check.mjs message --file <path>`, `node scripts/release_check.mjs versions`, `git diff --cached --check`, and relevant checks. Report commit SHA and actual validation. Existing authorization governs commit/push/merge/release. OpenCode commit tasks load `argos-commit` or `/ecc-commit`; `plan` prepares message/release-impact only, parent `build` owns authorized staging/commits, and editors report compatibility impact without committing. PR titles use the same convention; keep breaking footer in squash body.

## Project-specific contracts

- State lives under `~/.argos` (`ARGOS_HOME` overrides): SQLite `argos.db`, optional `memory_lancedb/`, `config.toml`, `auth.json`, and `hardware.json`. Migrations are additive/versioned; reopening preserves unrelated tables. `ARGOS_EMBED=0` uses Jaccard recall; `ARGOS_EMBED_MODEL_DIR` overrides the MiniLM cache.
- Roles, providers, Recon, Intel, Atlas, retry, and ID contracts are documented in [architecture](docs/architecture.md), [providers](docs/providers.md), [concepts](docs/concepts.md), and [usage](docs/usage.md). Tool bindings source of truth: `crates/argos-osint-core/src/recon/investigation/tool_io.rs`; provider adapters: `crates/argos-osint-core/src/osint/providers.rs`.
- Tests run offline with `ARGOS_EMBED` unset; MiniLM/LanceDB tests are ignored and require `ARGOS_EMBED=1`. Coverage tests ensure each tool input maps to a binding kind with prompt/rule extractors and a producer; fixtures cover each OSINT route's URL, method, headers, and host lock.
- Atlas cycles follow [the classification/Related contract](docs/atlas-classification-and-related-articles.md): Classifier-only collection-v1, native metadata, tentative Related leads; preserve independent Intel/Recon publication and historical evidence.
- New or significant architecture/concept changes update relevant docs and diagrams in the same change. Follow [architecture/concept conventions](docs/conventions.md#architecture-and-concepts): assess ownership, flow, persistence, public contracts, and terminology; update inaccurate figures or add focused ones, keep HTML/SVG/catalog/prose links aligned, and verify claims against final code including failures/retries. Internal edits preserving contracts need no new figure. Diagram style: [docs/diagrams.md](docs/diagrams.md) (HTML + inline SVG, orthogonal connectors, no shadows/Mermaid, accents on at most two nodes).
