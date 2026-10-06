# Argos OSINT — Agent Instructions

## Project Structure
Rust workspace with two crates:
- `crates/argos-osint-core` — core library: Brain (memory/recall), OSINT registry/executor, provider routing, Recon orchestration, SQLite + LanceDB persistence
- `crates/argos-osint-bin` — CLI (`argos`) and ratatui TUI terminal interface

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
| `argos.db` | SQLite: Brain memories, threads, runs, calls, cache, entities, provenance |
| `memory_lancedb/` | LanceDB vectors for Brain (table `brain_memories`, 384-dim) |
| `config.toml` | Role defaults (Recon, Tool picker, Synthesis, Classifier, Summarization), OSINT settings, recon limits |
| `auth.json` | Provider credentials (Grok, OpenAI, OpenRouter) — owner-only perms on Unix |
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

## Architecture Notes (non-obvious)
- **Three model roles** configured independently in Providers → Defaults: **Recon** (questions/bindings), **Tool picker** (tool ordering), **Synthesis** (answers). Old Writer role seeds Recon+Synthesis on migration; Tool picker defaults to OpenRouter `typesafe/jev-1.13` (decisions transport) only when empty.
- **Tool picker** has two transports: Jev decisions models (`typesafe/jev-*`) use OpenRouter `/alpha/decisions` API; others use chat transport with JSON repair.
- **Primary providers**: Firecrawl, SociaVault, Hunter. All other tools are gap-fillers. Hunter inputs only accept bindings from prompt, Firecrawl, SociaVault, or earlier Hunter calls.
- **Recon turn flow**: Brain recall → directives (d1–d5) → tool picker (1 pick/request, up to 13) → binder grounds every input → executor runs sequentially → streaming synthesis with deadline.
- **Atlas** is a separate Home app: two-phase news pipeline (GNews/NewsData discovery → NewsAPI/Currents headlines), daily quota ledger per provider.

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

## CI (`.github/workflows/ci.yml`)
Runs on 4 targets: macOS Intel (macos-15-intel), macOS ARM (macos-14), Linux x86_64 (ubuntu-24.04), Linux ARM64 (ubuntu-24.04-arm). Steps: `cargo fetch`, `cargo build --locked`, `cargo test --workspace --locked`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, plus ignored MiniLM test with `ARGOS_EMBED=1`.

## Key Files to Read for Context
- `README.md` — full product tour, navigation, CLI reference
- `docs/architecture.md` — investigation flow, tool picker, bindings, persistence, Atlas, news/legal context
- `docs/providers.md` — provider setup, model roles, OSINT provider details
- `crates/argos-osint-core/src/osint/providers.rs` — primary provider adapters
- `crates/argos-osint-core/src/recon/investigation/tool_io.rs` — tool input/binding table (source of truth for binder/picker)

## graphify

This project tracks a code and Cargo dependency graph at `graphify-out/`. OpenCode V2 uses project plugins in `.opencode/plugins/` for graph-first navigation, GSD hook compatibility, and a small session handoff. Use GSD for phases and milestones; use `/ecc-plan`, `/ecc-review`, `/ecc-verify`, `/ecc-checkpoint`, and `/ecc-learn` for focused workflow steps.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost). Track only `graph.json`, `manifest.json`, and `GRAPH_REPORT.md`; leave generated caches and visualization ignored.
