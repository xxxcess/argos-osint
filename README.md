# Argos OSINT

Argos is a terminal investigation workspace. Its launcher contains **Recon**, **Brain**, **OSINT**, **Providers**, and **System**, in that order. Recon starts by default and restores the last selected surviving thread.

## Run and navigate

```sh
cargo run -p argos-osint-bin
```

F1–F5 open the five apps. Tab and Shift+Tab move focus; Enter activates a focused control. Ctrl+N starts a Recon thread. Alt+Left and Alt+Right navigate recently opened threads. Shift+Enter adds a line in the composer; Enter sends. Esc closes a detail panel or returns focus without cancelling a run. Ctrl+C quits. Mouse selection, buttons, tabs, and scrolling are supported. On narrow terminals, panels are stacked.

Recon keeps threads, drafts, messages, run stages, plans, tool calls, and evidence across restarts. Its transcript shows cited answers and tool activity. Use the explicit Cancel action to stop a run; interrupted runs can be resumed. The composer accepts `:rename <title>`, `:delete`, and `:delete-with-insights` for the selected thread.

OSINT lists 30 HTTP lookup tools in ten categories. Choose a tool to see its input schema, example, documentation, access restrictions, result, and source. Manual runs remain in history and can be attached to a Recon thread without another request. `:prev` and `:next` navigate saved manual results. A standalone result does not create a Brain insight; Recon extracts insights from cited evidence after synthesis. Some public services require an identifying User-Agent and impose quotas or licensing limits. Set one before SEC or Nominatim lookups:

```sh
cargo run -p argos-osint-bin -- osint user-agent 'Argos contact@example.com'
```

Brain retains manual save, recall, pin, edit, and delete. Investigation insights have entity and topic anchors, evidence sources, and merged provenance. Providers keeps Grok, OpenAI, and OpenRouter connections, plus independent **Recon** and **Synthesis** choices under **Defaults**. Account sign-in does not change either default. System shows hardware and storage paths.

## CLI

Use `cargo run -p argos-osint-bin --` before these commands when running from source:

```sh
argos recon new --title 'Example investigation'
argos recon list --search example
argos recon show <thread-id>
argos recon ask <thread-id> 'What is known about example.org?'
argos recon ask-new 'What is known about example.org?'
argos recon resume <run-id>
argos recon retry <run-id>
argos recon delete <thread-id> --with-insights
argos osint list
argos osint describe shodan_internetdb
argos osint run shodan_internetdb --input '{"ip":"8.8.8.8"}'
argos osint history
argos osint attach <call-id> <thread-id>
argos defaults show
argos defaults set recon --provider openrouter --model <model-id>
argos defaults set synthesis --provider openrouter --model <model-id>
argos models --role recon
argos insights --entity example.org
argos remember --app research --conversation thread-123 'A manually saved fact'
argos recall 'What do I know?'
```

`recon delete` removes the thread and its contribution links. The default keeps uniquely sourced insight text; `--with-insights` removes unsupported extracted insights while preserving user pinned or edited ones. Shared insights keep surviving sources. `recon limits` shows or changes the per-turn round, call, and time budgets. `osint enable` and `osint disable` control tools. CLI commands return JSON where practical and use the same store, registry, and executor as the TUI.

## State and migration

State lives in `~/.argos`; `ARGOS_HOME` overrides the directory. `argos.db` holds Brain memories, threads, runs, calls, bounded cached responses, entities, and provenance. `config.toml` holds role defaults and OSINT settings. `auth.json` holds provider credentials with owner only permissions on Unix; `hardware.json` caches the host profile.

Opening the database performs additive versioned migrations. Existing Brain memories and unrelated tables are preserved. Old Writer settings seed the Synthesis default once; subsequent loads preserve both role choices. A run interrupted by process exit keeps completed observations and is marked interrupted on the next TUI launch. Resume explicitly continues remaining steps; retry starts a new turn.

## Limits and verification

The tool adapters issue bounded public HTTP requests, but public upstream availability and quotas can change. A registry entry or fixture test does not establish current live availability. Responses are observations with retrieval times, not proof of current ownership, personal identity, or exploitability. Shodan InternetDB's free access is restricted to noncommercial use. Public Nominatim requires an identifying User-Agent, attribution, caching, and a maximum of one request per second. Provider model calls use the account and terms you configure. [Architecture](docs/architecture.md) and [provider setup](docs/providers.md) give more detail.

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
