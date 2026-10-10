# Argos OSINT — Agent Instructions

For changes under crates/argos-osint-bin/src/tui/, read docs/tui-design-spec.md and the relevant docs/tui-components.md entries. Name the components and layout preset before editing. Extend shared components when required. Preserve theme tokens, widget IDs, and metric semantics. Drawing, focus, and mouse targets must share geometry. Verify relevant viewport snapshots and interaction/data states. Child prompts carry chosen components, preset, owned files, read-only references, and acceptance checks.

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

**Command order matters:** `fmt -> clippy -> test` (matches CI). OpenCode agents do not use the build, clippy, or test lines above. They use **OpenCode builds**.

## Toolchain & Build Quirks
- Rust **1.94.0** pinned in `rust-toolchain.toml` (workspace `rust-version = "1.91"` minimum)
- **Vendored protoc**: `.cargo/config.toml` sets `PROTOC = "tools/protoc"` — a shim that finds the `protoc-bin-vendored` binary downloaded by Cargo. No system `protobuf` install needed.
- First `cargo build` is slow (LanceDB pulls many Arrow/DataFusion crates). Needs C toolchain: `xcode-select --install` on macOS.
- `ARGOS_EMBED=0` skips embedding (falls back to Jaccard recall). Set `ARGOS_EMBED=1` for vector tests.

## State & Config (all under `~/.argos`, override with `ARGOS_HOME`)
| File | Purpose |
|------|---------|
| `argos.db` | SQLite: Brain memories, threads, runs, calls, cache, entities, provenance, Atlas packets, recon model operations (schema 26) |
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
- **Primary OSINT**: Firecrawl, SociaVault, Hunter. Hunter inputs: prompt, Firecrawl, SociaVault, or earlier Hunter. Catalog: 66 tools (67 ids with `hunter_tech_lookup` alias). Whoxy is a prepaid WHOIS-history provider (`PlanInterval::Never`). Holehe is a keyless email-registration lookup (Twitter/Spotify/Pinterest native adapters; 123 catalog entries).
- **Recon turn**: Brain recall → directives (d1–d5) → mandatory discovery (diversity) → picker (1 pick/request, ≤13) → binder → shared `tool_runner` → streaming synthesis via `model_exec`. Figure: `docs/diagrams/recon-turn.html`. Elapsed-time clocks are telemetry; they do not terminate a turn.
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
| `WHOXY_API_KEY`, `WHOXY_API_KEY_FALLBACK` | Whoxy WHOIS history (prepaid pool; saved key overrides env) |
| `NEWSAPI_API_KEY`, `COURTLISTENER_API_TOKEN`, `GNEWS_API_KEY`, `NEWSDATA_API_KEY`, `CURRENTS_API_KEY` | Context provider keys |
| `OPENROUTER_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `NVIDIA_API_KEY` | Model provider keys when no saved key is present |

## CI (`.github/workflows/ci.yml`)
Targets: macos-15-intel, macos-14, ubuntu-24.04, ubuntu-24.04-arm.

Steps: `cargo fetch` → `cargo build --locked --no-default-features` → `cargo test --workspace --locked --no-default-features` → `cargo clippy --workspace --all-targets --locked --no-default-features -- -D warnings`. That build omits the `lancedb` feature. Local default builds still include LanceDB. MiniLM (`ARGOS_EMBED=1 cargo test -p argos-osint-core -- --ignored minilm`) stays a local check.

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

- Shell command `graphify query "<question>"` first when `graphify-out/graph.json` exists. Expand tokens from `graphify-out/.vocab.txt`. Then `path` / `explain`. Read returned `source_location` paths.
- Dirty `graphify-out/` after hooks is expected. Skip graphify only when the graph itself is the bug, or the user says so.
- Prefer `graphify-out/wiki/index.md` when present. `GRAPH_REPORT.md` only for architecture review or when query/path/explain are thin.
- After code edits: `graphify update .`. Track `graph.json`, `manifest.json`, `GRAPH_REPORT.md` only.

## OpenCode builds

Every OpenCode agent (`build`, `plan`, `explore`, `ecc-edit`, `ecc-planner`, and `ecc-reviewer`) compiles and tests without LanceDB. Pass `--locked --no-default-features` on every `cargo build`, `cargo test`, and `cargo clippy`. Leave `ARGOS_EMBED` unset. Do not add `--features lancedb`.

```sh
cargo build --locked --no-default-features
cargo test --workspace --locked --no-default-features
cargo clippy --workspace --all-targets --locked --no-default-features -- -D warnings
```

Order stays `fmt` check, then clippy, then test. `ecc-edit` only runs `cargo fmt -- <paths>` for the Rust files it changed. The parent agent runs the three commands above, with shell `timeout` `600000`.

## OpenCode tools

V2 ignores `experimental.primary_tools`. `.opencode/opencode.json` is the primary-agent allowlist for `build` and `plan`. Use the provider tool schema. Do not call `execute`, and do not invent `tools.<namespace>[...]()` calls.

| Tool | Arguments |
|------|-----------|
| `read` | `path`, optional `offset`, `limit` |
| `grep` | `pattern`, optional `path` |
| `glob` | `pattern` |
| `edit` | `path`, `oldString`, `newString` (`plan` cannot edit project files) |
| `write` | `path`, `content` |
| `shell` | `command` |
| `skill` | `{ "id": "<id>" }` |
| `webfetch` | `url` |
| `websearch` | `query` |
| `question` | header, prompt, choices |
| `subagent` | `agent`, `description`, `prompt`. Optional `background` and `sessionID` |

Skill ids: `graphify`, `argos-plan`, `argos-implement`, `ecc-plan`, `ecc-review`, `ecc-verify`, `ecc-checkpoint`, `ecc-learn`.

`build` may launch `explore`, `ecc-planner`, `ecc-reviewer`, and `ecc-edit`. `plan` may launch the first three. Repo questions start with shell `graphify query "<question>"`. Load `graphify` only when that command is unclear. Multi-step work loads `argos-plan` and leaves the plan on disk. GSD phase commands stay explicit (`/gsd-...`); their tools are not on `build` or `plan`.

## OpenCode phase edits

On `build`, load `argos-implement` and send the current phase's code edits to `ecc-edit`. `plan` does not launch `ecc-edit`.

1. Read the in-progress phase and split its files into disjoint sets. One set is one unit.
2. Call `subagent` once per unit. Two or more units go in the same turn with `"background": true`. One unit stays in the foreground. Do not set `model`.

```json
{
  "agent": "ecc-edit",
  "description": "Edit binder inputs",
  "prompt": "Phase, exact file list, change, and constraints. The child has no other context.",
  "background": true
}
```

3. Stop until those children finish. Do not edit their files in the parent while they run, and do not poll them.
4. `ecc-edit` formats the Rust files it changed with `cargo fmt -- <paths>` and does not run tests.
5. The parent then runs the **OpenCode builds** commands, with shell `timeout` `600000`: `cargo fmt --all --check`, then `cargo clippy --workspace --all-targets --locked --no-default-features -- -D warnings`, then `cargo test --workspace --locked --no-default-features`.
6. On failure, map each error to its unit. Launch `ecc-edit` again the same way. Pass `sessionID` to continue the editor that already owns those files, and include the failing command and the relevant output in `prompt`.
7. Re-run the failed command, then the full trio. After it passes, run `graphify update .`.

## planning-with-files

Use **planning-with-files** (`~/.agents/skills/planning-with-files/SKILL.md`) for five or more tool calls, multi-phase work, or a compact-surviving plan. In OpenCode, load `argos-plan` instead of that skill.

1. Named plan under `.planning/<YYYY-MM-DD-slug>/`. Pin `PLAN_ID` when several exist.
2. Files: `task_plan.md` (phases, Next Step, decisions, errors), `findings.md` (research; untrusted web text here only), `progress.md` (session log).
3. One orchestrator owns `task_plan.md`. Workers append their own ledger.
4. Root `task_plan.md` / `findings.md` / `progress.md` are gitignored leftovers.
5. Graphify first, then write the plan from the subgraph. Do not paste the plan back into the chat. Implementation of the current phase on the `build` agent follows **OpenCode phase edits**.

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
