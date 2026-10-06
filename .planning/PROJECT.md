# PROJECT.md — argos-osint

## Project Purpose

Argos is a terminal investigation workspace for OSINT (Open Source Intelligence). It provides a structured investigation environment with Brain (memory/recall), Intel (news/pipeline), Atlas (two-phase news), and Recon (chat-based investigation) applications.

## Core Components

### Crates

- **`argos-osint-core`**: Core library — Brain (memory/recall), OSINT registry/executor, provider routing, Recon orchestration, SQLite + LanceDB persistence
- **`argos-osint-bin`**: CLI (`argos`) and ratatui TUI terminal interface

### Applications

- **Home**: Launches Intel, Atlas, Brain, and Recon. TUI entry point.
- **Intel**: Browses Atlas-stored headlines with classification, search, and country mini-map
- **Atlas**: Two-phase news pipeline (GNews/NewsData discovery → NewsAPI/Currents headlines), daily quota ledger per provider
- **Brain**: Retains manual save, recall, pin, edit, delete. Investigation insights have entity and topic anchors.
- **Recon**: Threads, drafts, messages, run stages, plans, tool calls, evidence, memories. Chat-based investigation with model-driven tool picking.

## Investigation Flow

1. Brain recall → directives (d1–d5) → tool picker (1 pick/request, up to 13) → binder grounds every input → executor runs sequentially → streaming synthesis with deadline
2. Tool picker orders tools with classified Recon mode and section priorities
3. Every input records grounding: `{step, input, value, source}` where source is directive entity, prompt, accepted binding, or fixed value
4. Primary providers: Firecrawl, SociaVault, Hunter. All other tools are gap-fillers.

## State Directory (`~/.argos`)

| File | Purpose |
|------|---------|
| `argos.db` | SQLite: Brain memories, threads, runs, calls, cache, entities, provenance |
| `memory_lancedb/` | LanceDB vectors for Brain (table `brain_memories`, 384-dim) |
| `config.toml` | Role defaults (Recon, Tool picker, Synthesis), OSINT settings, recon limits |
| `auth.json` | Provider credentials (Grok, OpenAI, OpenRouter) — owner-only perms on Unix |
| `hardware.json` | Cached host profile |

## CLI Entry Points

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