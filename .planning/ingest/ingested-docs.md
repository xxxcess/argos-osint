# Docs Ingest — argos-osint

## Ingested Documentation

The following documentation files have been ingested into the planning workspace:

### `docs/architecture.md`

- **Pages:** Investigation flow, tool picker, bindings, persistence, Atlas, news/legal context
- **Key sections:**
  - Turn flow: Brain recall → directives (d1–d5) → tool picker (1 pick/request, up to 13) → binder grounds every input → executor runs sequentially → streaming synthesis with deadline
  - Tool inputs and bindings: `recon/investigation/tool_io.rs` holds one row per catalog tool (`TOOLS`): binding kind per input, how written, optional fills, fixed extras, declared producers, kinds produced, JSON keys
  - Primary providers: Firecrawl, SociaVault, Hunter. All other tools are gap-fillers.
  - Tool picker transports: Jev decisions models use OpenRouter `/alpha/decisions` API; others use chat transport with JSON repair
  - Coverage tests: every tool input maps to a binding kind with prompt extractor, rule extractor, and producer
  - Fixture tests: request builders for every OSINT route (URL, method, headers, host lock)
  - Primary data flow: Brain recall → directives → tool picker → binder → executor → synthesis
  - Persistence: `store.rs` additive migrations, `schema_recon.sql` for threads, messages, runs, calls, etc.
  - Brain recall: hybrid (SQLite source of truth + LanceDB vectors), MiniLM embeddings (384-dim)
  - Persistence: `store.rs` migrates existing Brain database additively, then applies `schema_recon.sql`
  - Model and UI boundaries: Provider accounts in `auth.json`, Recon/Tool picker/Synthesis choices in `config.toml`
  - Atlas: two-phase news pipeline (GNews/NewsData discovery → NewsAPI/Currents headlines), daily quota ledger
  - News and Legal context tools: `newsapi_search`, `newsapi_headlines`, `courtlistener_case_search`, etc.
  - Admiralty source evaluation (WP:RSP): Source Reliability A–F, Information Credibility 1–6

### `docs/providers.md`

- **Pages:** Provider defaults, OSINT data providers
- **Key sections:**
  - Three model roles configured independently in Providers → Defaults: Recon, Tool picker, Synthesis
  - Old Writer role seeds Recon+Synthesis on migration; Tool picker defaults to OpenRouter `typesafe/jev-1.13` when empty
  - Tool picker has two transports: Jev decisions models use OpenRouter `/alpha/decisions` API; others use chat transport with JSON repair
  - Primary providers: Firecrawl, SociaVault, Hunter. All other tools are gap-fillers.
  - Hunter inputs only accept bindings from prompt, Firecrawl, SociaVault, or earlier Hunter calls
  - NewsAPI and CourtListener are keyed context providers, not primary providers
  - Provider credentials stored in `~/.argos/auth.json` with owner-only permissions on Unix
  - API keys: `FIRECRAWL_API_KEY`, `SOCIAVAULT_API_KEY`, `HUNTER_API_KEY`
  - Context provider keys: `NEWSAPI_API_KEY`, `COURTLISTENER_API_TOKEN`, `GNEWS_API_KEY`, `NEWSDATA_API_KEY`, `CURRENTS_API_KEY`
  - `argos login` / `argos logout` for provider connections
  - `argos defaults show` / `argos defaults set` for role/configuration

## Ingest Status

- All documentation files successfully ingested
- No conflicts detected
- Content verified against codebase structure