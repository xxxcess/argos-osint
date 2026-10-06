# REQUIREMENTS.md — argos-osint

## Functional Requirements

### Recon Investigations

- `recon new --title <title>`: Create a new investigation thread
- `recon ask <thread-id> <question>`: Ask a question about an investigation
- `recon ask-new <question>`: Start a new investigation and ask a question
- `recon resume <run-id>`: Resume an interrupted run
- `recon retry <run-id>`: Retry a run (starts a new turn)
- `recon show <thread-id>`: Show investigation transcript with plan, directives, tool calls, evidence
- `recon delete <thread-id> --with-insights`: Delete investigation and its Brain memories

### OSINT Manual Runs

- `osint list`: List available OSINT tools
- `osint describe <tool-id>`: Describe a tool (input schema, example, documentation)
- `osint run <tool-id> --input '{"key":"value"}'`: Run a tool manually
- `osint history`: Show history of manual OSINT runs
- `osint attach <call-id> <thread-id>`: Attach a manual run to a thread
- `osint enable|disable <tool-id>`: Enable/disable a tool
- `osint user-agent 'Contact <email>'`: Set user-agent for public HTTP services

### Provider & Model Defaults

- `defaults show`: Show current role defaults
- `defaults set recon|tool-picker|synthesis --provider <p> --model <m>`: Set role default
- `models --role <role>`: List models for a role

### Brain / Memory

- `insights --entity <name>`: Look up insights about an entity
- `remember --app <app> --conversation <id> 'fact'`: Manually save a fact
- `recall 'query'`: Recall memories matching a query
- `memories`: List Brain memories
- `memories reindex`: Rebuild LanceDB index

### Atlas (News Pipeline)

- Atlas discovery and resume/pause operations

### System

- `argos defaults show`
- `argos defaults set`
- `argos models --role`
- Hardware and storage path display

### Non-Goals

- No active scanning or shell execution
- No per-host rate limiting beyond configured caps
- No shell execution in OSINT tools

## Non-Functional Requirements

- **Offline tests**: Workspace tests run offline (`ARGOS_EMBED` unset)
- **Embedding optional**: `ARGOS_EMBED=0` skips embedding, falls back to Jaccard recall
- **Rust 1.94.0**: Pinned in `rust-toolchain.toml`, workspace `rust-version = "1.91"` minimum
- **Vendored protoc**: `.cargo/config.toml` sets `PROTOC = "tools/protoc"` — no system protobuf needed
- **C toolchain**: `xcode-select --install` on macOS for first build (LanceDB pulls many Arrow/DataFusion crates)

## Configuration Requirements

### Provider Keys (required for OSINT)

- `FIRECRAWL_API_KEY` — Firecrawl bearer token
- `SOCIAVAULT_API_KEY` — SociaVault X-API-Key
- `HUNTER_API_KEY` — Hunter x-api-key

### Context Provider Keys

- `NEWSAPI_API_KEY` or `newsapi_api_key` — NewsAPI
- `COURTLISTENER_API_TOKEN` or `courtlistener_api_token` — CourtListener
- `GNEWS_API_KEY` or `gnews_api_key` — GNews
- `NEWSDATA_API_KEY` or `newsdata_api_key` — NewsData
- `CURRENTS_API_KEY` or `currents_api_key` — Currents

### User-Agent

- `argos osint user-agent 'Contact <email>'` — required for Nominatim/SEC lookups

## Success Criteria

- All CLI commands function correctly per specification
- Coverage tests pass: every tool input maps to a binding kind with prompt extractor, rule extractor, and producer
- Fixture tests pass: request builders for every OSINT route (URL, method, headers, host lock)
- Brain recall (hybrid Jaccard + vector) works offline and with embeddings
- Atlas two-phase pipeline respects daily quota ledgers