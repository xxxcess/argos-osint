# Codebase Map — argos-osint

## Crates

### `argos-osint-core` — Core Library

**Purpose:** Durable state, shared OSINT registry/executor, provider routing, Recon orchestration, SQLite + LanceDB persistence.

**Key Modules:**

- `brain.rs` — Memory storage, recall, hybrid recall (Jaccard + vector), `remember`/`recall` CLI commands
- `osint.rs` — OSINT tool registry, `TOOLS` catalog, `osint::registry()`, provider adapters in `osint/providers.rs`
- `recon.rs` — Recon orchestration: turn flow, directive extraction, tool picker, binder, executor, streaming synthesis
- `tool_io.rs` — **Source of truth for binder/picker**: one row per catalog tool (`TOOLS`): binding kinds per input, producers, kinds produced, rule extractors, extractor logic. Coverage tests assert every required input maps to a binding kind with prompt extractor, rule extractor, and producer.
- `provider.rs` — Provider trait and routing logic
- `provider_request.rs` — Request building and JSON repair
- `provider_diag.rs` — Provider diagnostics
- `provider_attempt.rs` — Provider attempt state
- `scheduler.rs` — Turn deadline, per-call timeouts, concurrency cap, retries
- `summarization.rs` / `summarization/exec.rs` — Synthesis orchestration, deadline management, streaming
- `brain_lance.rs` — LanceDB vector wrapper (sync API over async), `memory_lancedb/` persistence
- `embed.rs` — MiniLM embedding (384-dim, mean-pooling + L2 norm, Xenova ONNX via tract)
- `evidence.rs` — Evidence tracking, provenance, entity/claim anchoring
- `store.rs` — SQLite schema management, additive migrations (`schema_recon.sql`), versioned tables: threads, messages, runs, calls, cache, settings, entities, claim insights, sources, relations, edits, extraction jobs, app state
- `intel_recon/` — Investigation flow: directives (d1–d5), modes (Verify/Explain/Assess Outlook), body, classify_mode, jobs, ledger, persist, replace_insights
- `atlas.rs` / `atlas_memory.rs` / `atlas_insights.rs` — Atlas two-phase news pipeline (GNews/NewsData discovery → NewsAPI/Currents headlines), daily quota ledger
- `osint/atlas_news.rs` — Atlas discovery phase tools
- `osint/news_legal.rs` — NewsAPI/CourtListener context tools
- `osint/source_eval.rs` — Source evaluation
- `related_memories.rs` — Related memory lookup
- `hardware.rs` — Host profile caching
- `iso3166.rs` — Country code utilities
- `pipeline.rs` — Job pipeline orchestration
- `tasks.rs` — Task management
- `graph_explanation.rs` — Graph explain functionality
- `reliability_faults.rs` — Reliability/fault tracking

**Key Data Structures:**

- `TOOLS` — Static catalog of 59 tools across 15 categories
- `PROVIDERS` — Firecrawl, SociaVault, Hunter as primary; NewsAPI, CourtListener, GNews, NewsData, Currents as context
- `DIRECTIVES` — d1–d5, each with entity, targets, directive kind
- `ToolIo` — Per-input binding kind, producer, extractor, JSON key

### `argos-osint-bin` — CLI and TUI

**Purpose:** CLI entry point (`argos` binary) and terminal user interface.

**Key Files:**

- `main.rs` — CLI argument parsing, entry point
- `cli.rs` — CLI command implementations (recon, osint, defaults, brain, memories)
- `tui/app.rs` — TUI application state, focus management, event handling
- `tui/ui.rs` — Rendering: controls, Recon transcript, matching hit regions, theme
- `tui/map.rs` — TUI map view
- `tui/jobs.rs` — Job status display in TUI
- `tui/logs.rs` — Event log display
- `tui/summary_card.rs` — Summary card rendering
- `tui/graph.rs` — Graph view rendering
- `tui/land.rs` — Land view (Home screen)
- `tui/brain_detail.rs` — Brain memory detail view

**CLI Commands (entry points):**

```sh
# Recon
argos recon new --title '...'
argos recon list --search <term>
argos recon show <thread-id>
argos recon ask <thread-id> 'question'
argos recon ask-new 'question'
argos recon resume <run-id>
argos recon retry <run-id>
argos recon delete <thread-id> --with-insights

# OSINT
argos osint list
argos osint describe <tool-id>
argos osint run <tool-id> --input '{"key":"value"}'
argos osint history
argos osint attach <call-id> <thread-id>
argos osint enable|disable <tool-id>
argos osint user-agent 'Contact <email>'

# Provider defaults
argos defaults show
argos defaults set recon|tool-picker|synthesis --provider <p> --model <m>
argos models --role recon|tool-picker|synthesis

# Brain
argos insights --entity <name>
argos remember --app <app> --conversation <id> 'fact'
argos recall 'query'
argos memories
argos memories reindex
```

## Primary Data Flow

1. **Brain recall** → directives (d1–d5)
2. **Tool picker** (1 pick/request, up to 13) → binder grounds every input
3. **Executor** runs sequentially → streaming synthesis with deadline
4. Every input records grounding: `{step, input, value, source}`
5. Source is directive entity, prompt, accepted binding, or fixed value

## State & Config (all under `~/.argos`)

| File | Purpose |
|------|---------|
| `argos.db` | SQLite: Brain memories, threads, runs, calls, cache, entities, provenance |
| `memory_lancedb/` | LanceDB vectors for Brain (table `brain_memories`, 384-dim) |
| `config.toml` | Role defaults (Recon, Tool picker, Synthesis), OSINT settings, recon limits |
| `auth.json` | Provider credentials (Grok, OpenAI, OpenRouter) — owner-only perms on Unix |
| `hardware.json` | Cached host profile |

## Tool Input & Binding Kinds (from `tool_io.rs`)

The catalog has 59 tools with binding kinds: domain, ip, email, url, handle, platform_id, person_name, org_name, cve, package, wallet, address, coordinates. Each kind has a prompt extractor, rule extractor, and at least one tool producer (coverage test assertion).

**Primary providers:** Firecrawl, SociaVault, Hunter. All other tools are gap-fillers.

**Tool picker transports:** Jev decisions models use OpenRouter `/alpha/decisions` API; others use chat transport with JSON repair.

## Documentation Ingest

See `.planning/ingest/` for ingested docs.