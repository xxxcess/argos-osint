# Argos OSINT — Agent Instructions

## Project Structure
Rust workspace with two crates:
- `crates/argos-osint-core` — core library: Brain (memory/recall), OSINT registry/executor, provider routing, Recon orchestration, SQLite + LanceDB persistence
- `crates/argos-osint-bin` — CLI (`argos`) and ratatui TUI terminal interface

Figures: [workspace](docs/diagrams/workspace.html) · [persistence](docs/diagrams/persistence.html) · [catalog](docs/diagrams.md)

## Developer Commands
```sh
# Run the TUI
cargo run -p argos-osint-bin

# Build / check
cargo build
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings

# Test (offline; ARGOS_EMBED unset)
cargo test --workspace

# Integration test with MiniLM embeddings (downloads ~23 MB model on first run)
ARGOS_EMBED=1 cargo test -p argos-osint-core -- --ignored minilm
```

**Command order matters:** `fmt -> clippy -> test` (matches CI)

## Toolchain & Build Quirks
- Rust **1.94.0** pinned in `rust-toolchain.toml` (workspace `rust-version = "1.91"` minimum)
- **Vendored protoc**: `.cargo/config.toml` sets `PROTOC = "tools/protoc"` — a shim that finds the `protoc-bin-vendored` binary downloaded by Cargo. No system `protobuf` install needed.
- First `cargo build` is slow (LanceDB pulls many Arrow/DataFusion crates). Needs C toolchain: `xcode-select --install` on macOS.
- `ARGOS_EMBED=0` skips embedding (falls back to Jaccard recall). Set `ARGOS_EMBED=1` for vector tests.

## State & Config (all under `~/.argos`, override with `ARGOS_HOME`)
| File | Purpose |
|------|---------|
| `argos.db` | SQLite: Brain memories, threads, runs, calls, cache, entities, provenance, Atlas packets (schema 25) |
| `memory_lancedb/` | LanceDB vectors for Brain (table `brain_memories`, 384-dim) |
| `config.toml` | Role defaults (Recon, Tool picker, Synthesis, Classifier, Summarization), OSINT settings, recon limits |
| `auth.json` | Provider credentials (OpenRouter, Google, Nvidia, and preserved legacy Grok/OpenAI accounts) — owner-only perms on Unix |
| `hardware.json` | Cached host profile |

Migrations are additive and versioned; reopening preserves new/unrelated tables.

## CLI Entry Points (`argos` binary)
```sh
# Recon (investigations)
argos recon new --title '...'
argos recon list --search <term>
argos recon show <thread-id>
argos recon ask <thread-id> 'question'
argos recon ask-new 'question'
argos recon resume <run-id>
argos recon retry <run-id>
argos recon delete <thread-id> --with-insights

# OSINT (manual tool runs)
argos osint list
argos osint describe <tool-id>
argos osint run <tool-id> --input '{"key":"value"}'
argos osint history
argos osint attach <call-id> <thread-id>
argos osint enable|disable <tool-id>
argos osint user-agent 'Contact <email>'   # required for Nominatim/SEC

# Provider / model defaults
argos defaults show
argos defaults set recon|tool-picker|synthesis|classifier|summarization --provider <p> --model <m>
argos models --role recon|tool-picker|synthesis|classifier|summarization

# Brain
argos insights --entity <name>
argos remember --app <app> --conversation <id> 'fact'
argos recall 'query'
argos memories
argos memories reindex   # rebuild LanceDB index
```

## Architecture notes

- **Roles** (Models → Defaults): Recon, Tool picker, Synthesis, Classifier, Summarization, plus harness roles. Ordered `fallbacks` per role. Old Writer seeds Recon+Synthesis. Tool picker seeds OpenRouter `typesafe/jev-1.13` only when empty.
- **Picker transports**: `typesafe/jev-*` → OpenRouter `/alpha/decisions`. Others → chat JSON + repair.
- **Primary OSINT**: Firecrawl, SociaVault, Hunter. Hunter inputs: prompt, Firecrawl, SociaVault, or earlier Hunter.
- **Recon turn**: Brain recall → directives (d1–d5) → picker (1 pick/request, ≤13) → binder → sequential executor → streaming synthesis. Figure: `docs/diagrams/recon-turn.html`.
- **Intel**: jobs on an Atlas article; `intel_report_attempts` for dispatch counts; Summary = selected `bluf`. Figure: `docs/diagrams/intel-report.html`.
- **Atlas**: GNews/NewsData discovery → NewsAPI/Currents headlines; phase 4 durable packets; phase 5 index/verify. Outcomes: completed / waiting / blocked / partial / failed. Optional context failures are warnings. Indexed +1 (brief) is expected.
- **Retries**: primary 4 attempts (10/20/30s waits), each fallback 3 (10/20s). One retry owner; `complete()` is one-shot streaming. Figure: `docs/diagrams/atlas-pipeline.html`.
- **IDs**: Tools=`Osint`, Models=`Providers`, Profile=`System`.

## Testing Quirks
- Workspace tests run offline (`ARGOS_EMBED` unset). MiniLM/LanceDB tests are `#[ignore]` and require `ARGOS_EMBED=1`.
- Coverage tests in `argos-osint-core` verify every tool input maps to a binding kind with prompt extractor, rule extractor, and producer.
- Fixture tests cover request builders for every OSINT route (URL, method, headers, host lock).

## Environment Variables
| Var | Effect |
|-----|--------|
| `ARGOS_HOME` | Override state directory (`~/.argos`) |
| `ARGOS_EMBED=0` | Disable embeddings, use Jaccard-only recall |
| `ARGOS_EMBED_MODEL_DIR` | Override MiniLM cache location |
| `FIRECRAWL_API_KEY`, `SOCIAVAULT_API_KEY`, `HUNTER_API_KEY` | Primary provider keys (saved keys override env) |
| `NEWSAPI_API_KEY`, `COURTLISTENER_API_TOKEN`, `GNEWS_API_KEY`, `NEWSDATA_API_KEY`, `CURRENTS_API_KEY` | Context provider keys |
| `OPENROUTER_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `NVIDIA_API_KEY` | Model provider keys when no saved key is present |

## CI (`.github/workflows/ci.yml`)
Targets: macos-15-intel, macos-14, ubuntu-24.04, ubuntu-24.04-arm.

Steps: `cargo fetch` → `cargo build --locked` → `cargo test --workspace --locked` → `cargo clippy --workspace --all-targets --locked -- -D warnings` → ignored MiniLM (`ARGOS_EMBED=1`).

## Key Files to Read for Context
- `docs/README.md` — documentation map
- `README.md` — product overview and doc index
- `docs/usage.md` — TUI navigation and CLI
- `docs/architecture.md` — investigation flow, tool picker, bindings, persistence, Atlas, Intel reports
- `docs/concepts.md` — glossary (module ids, roles, bindings, schema)
- `docs/conventions.md` — TUI, schema, tests, scratch, planning, diagrams
- `docs/providers.md` — provider setup, model roles, OSINT provider details
- `docs/diagrams.md` — editorial diagram language (diagram-design)
- `crates/argos-osint-core/src/osint/providers.rs` — primary provider adapters
- `crates/argos-osint-core/src/recon/investigation/tool_io.rs` — tool input/binding table (source of truth for binder/picker)

## graphify

Tracked graph: `graphify-out/`. Skill: `.opencode/skills/graphify/SKILL.md`. OpenCode V2 plugins: `.opencode/plugins/`. GSD for phases; `/ecc-plan`, `/ecc-review`, `/ecc-verify`, `/ecc-checkpoint`, `/ecc-learn` for focused steps.

Plan agent: wait for approval, then prefer a `build` subagent with the approved plan. Keep scope. Preserve unrelated working-tree changes.

`/graphify` → use the installed graphify skill first.

[![Agent exploration](docs/diagrams/agent-graphify.svg)](docs/diagrams/agent-graphify.html)

- `graphify query "<question>"` first when `graphify-out/graph.json` exists. Expand tokens from `graphify-out/.vocab.txt`. Then `path` / `explain`. Search returned `source_location` paths.
- Dirty `graphify-out/` after hooks is expected. Skip graphify only when the graph itself is the bug, or the user says so.
- Prefer `graphify-out/wiki/index.md` when present. `GRAPH_REPORT.md` only for architecture review or when query/path/explain are thin.
- After code edits: `graphify update .`. Track `graph.json`, `manifest.json`, `GRAPH_REPORT.md` only.

## planning-with-files

Use **planning-with-files** (`~/.agents/skills/planning-with-files/SKILL.md`) for five or more tool calls, multi-phase work, or a compact-surviving plan.

1. Named plan under `.planning/<YYYY-MM-DD-slug>/` (`scripts/init-session.sh "Task name"`). Pin `PLAN_ID` when several exist.
2. Files: `task_plan.md` (phases, Next Step, decisions, errors), `findings.md` (research; untrusted web text here only), `progress.md` (session log).
3. One orchestrator owns `task_plan.md`. Workers append their own ledger.
4. Root `task_plan.md` / `findings.md` / `progress.md` are gitignored leftovers.
5. Graphify first, then write the plan from the subgraph.

## diagrams

Figures in `docs/diagrams/` follow [cathrynlavery/diagram-design](https://github.com/cathrynlavery/diagram-design): HTML + inline SVG, orthogonal connectors, no shadows, no Mermaid, accent on at most two nodes. Conventions: `docs/diagrams.md`. Link new figures from `docs/README.md` and the matching prose doc.

## Temporary agent scripts

Scratch = agent-only helpers Argos does not need to build or test (`*.py`, `*.sh`, logs, temp `*.rs`).

- Location: `.agent-scratch/` only (gitignored; keep the ignore entry).
- Never next to `Cargo.toml` or inside `crates/`, `docs/`, `scripts/`.
- Never `git add` scratch files. `git rm` if one is already tracked.
- Prefer editor edits. No line-number rewrites (`sed -i 'N,Mc'`) without re-reading the file.
- After `fmt` → `clippy` → `test`, delete that task’s scratch files.
- Keep `scripts/` only for documented project utilities (`scripts/render_tui_cells.py`).
- Incomplete work: leave scratch in `.agent-scratch/` and list it in the hand-off.
