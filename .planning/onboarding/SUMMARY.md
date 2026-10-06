# Onboarding Summary — argos-osint

## Welcome to Arg OSINT Onboarding

This workspace has been successfully onboarded. Below is a summary of what was learned and the next steps.

### What Was Learned

#### Codebase Structure

The argos-osint codebase consists of two crates:

- **`argos-osint-core`** (core library): Owns durable state, the shared OSINT registry/executor, provider routing, and Recon orchestration. Key modules include brain (memory/recall), osint (tool registry and providers), recon (turn orchestration), tool_io (binder/picker source of truth), provider (routing logic), summarization (synthesis), store (SQLite migrations), and atlas (two-phase news pipeline).

- **`argos-osint-bin`** (CLI + TUI): Owns the CLI (`argos` binary) and ratatui terminal interface. Key files include main.rs (CLI entry point), cli.rs (command implementations), and tui/ (all terminal UI components: app, ui, map, jobs, logs, summary_card, graph, land).

#### Investigation Flow

Each Recon turn follows this sequence:

1. **Brain recall** — retrieves relevant memories (hybrid Jaccard + vector search)
2. **Directive extraction** — extracts d1–d5 directives from the user prompt
3. **Tool picker** — picks one tool per request (up to 13 total), using either Jev decisions API (OpenRouter) or chat transport
4. **Binder** — grounds every input using directive entities, prompt text, accepted bindings, or fixed values
5. **Executor** — runs tools sequentially through the same `osint::Executor`
6. **Streaming synthesis** — produces the final answer with cited evidence

#### Primary Providers

- **Firecrawl** (search, scrape, map, batch_scrape, crawl, extract) — bearer token auth
- **SociaVault** (profile, search, search_users, user_content, google_search) — X-API-Key auth, 44 one-credit routes
- **Hunter** (domain_finder, email_count, domain_search, email_finder, email_verifier, company_enrichment, email_insight, person_enrichment, combined_enrichment) — read-only, x-api-key header

All other tools are gap-fillers driven by primary provider data.

#### Model Roles (configured independently in Providers → Defaults)

- **Recon**: Derives turn's three questions, extracts input bindings from observations
- **Tool picker**: Picks one tool per request until the ordered list is complete
- **Synthesis**: Writes cited answers, extracts evidence-backed claims

Default: Tool picker uses OpenRouter `typesafe/jev-1.13` (decisions API).

#### State Directory (`~/.argos`)

- `argos.db` — SQLite: Brain memories, threads, runs, calls, cache, entities, provenance
- `memory_lancedb/` — LanceDB vectors for Brain (table `brain_memories`, 384-dim)
- `config.toml` — Role defaults (Recon, Tool picker, Synthesis), OSINT settings, recon limits
- `auth.json` — Provider credentials (Grok, OpenAI, OpenRouter) — owner-only perms on Unix
- `hardware.json` — Cached host profile

#### CLI Entry Points

**Recon:** `argos recon new --title '...'`, `argos recon list --search <term>`, `argos recon show <thread-id>`, `argos recon ask <thread-id> 'question'`, `argos recon ask-new 'question'`, `argos recon resume <run-id>`, `argos recon retry <run-id>`, `argos recon delete <thread-id> --with-insights`

**OSINT:** `argos osint list`, `argos osint describe <tool-id>`, `argos osint run <tool-id> --input '{"key":"value"}'`, `argos osint history`, `argos osint attach <call-id> <thread-id>`, `argos osint enable|disable <tool-id>`, `argos osint user-agent 'Contact <email>'`

**Defaults:** `argos defaults show`, `argos defaults set recon|tool-picker|synthesis --provider <p> --model <m>`, `argos models --role recon|tool-picker|synthesis`

**Brain:** `argos insights --entity <name>`, `argos remember --app <app> --conversation <id> 'fact'`, `argos recall 'query'`, `argos memories`, `argos memories reindex`

### Planning Artifacts Created

| Artifact | Path |
|---|---|
| Codebase map | `.planning/codebase/map.md` |
| Ingested docs | `.planning/ingest/ingested-docs.md` |
| Project brief | `.planning/PROJECT.md` |
| Requirements | `.planning/REQUIREMENTS.md` |
| Roadmap | `.planning/ROADMAP.md` |
| State summary | `.planning/STATE.md` |
| Onboarding summary | `.planning/onboarding/SUMMARY.md` |

### Next Commands

```sh
# Verify the build
cargo build

# Check formatting
cargo fmt --all --check

# Run clippy
cargo clippy --workspace --all-targets -- -D warnings

# Run offline tests
cargo test --workspace

# Run MiniLM embedding tests (requires ARGOS_EMBED=1)
ARGOS_EMBED=1 cargo test -p argos-osint-core -- --ignored minilm
```

### Verification

All planning artifacts have been created and verified against the live codebase. The onboard workflow follows the standard GSD order: codebase map → docs ingest → project initialization → onboarding summary.

---
**Onboarded**: 2026-10-06