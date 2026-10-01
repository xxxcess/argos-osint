# Argos OSINT

Argos is a terminal investigation workspace. It opens on **Home**. **Recon** and **Brain** are the two applications. **OSINT**, **Providers**, and **System** are system apps: they configure gathering, accounts, and host state. Only Recon has a chat. The last surviving Recon thread is restored when Recon opens.

## Run and navigate

```sh
cargo run -p argos-osint-bin
```

From Home, ↑↓ select and Enter opens. `1` opens Recon, `2` Brain, `3` OSINT, `4` Providers, and `5` System. Esc or the Home control leaves an app. `?` opens the shortcut card for the current screen.

Recon opens on a full-screen list of recent investigations. ↑↓ moves through that list and Enter opens the selected investigation as its own full-screen transcript. Esc from the transcript returns to the list; Esc from the list returns Home. In the transcript, Tab moves between the log and the prompt. Enter sends; Shift+Enter adds a line. While the transcript is focused, ↑↓ select a message, decision, or tool, and ←→ fold a decision or tool log. Enter toggles the selected fold. `f` opens the full text. Ctrl+U and Ctrl+D scroll the focused pane, and the mouse wheel scrolls the pane under the pointer. Ctrl+N starts an investigation and opens its transcript. In the transcript, when the cursor is not in a text field, Alt+Left and Alt+Right move through recently opened investigations. A click counts on release, so dragging across Home does not launch an app. Ctrl+C cancels a running turn, or clears a draft first. Press Ctrl+C or Ctrl+Q again within a second to quit when nothing else is pending.

The transcript follows the Grok Build chat. Your question sits on a raised band with a prompt arrow. The answer is rendered Markdown: headings, lists, bold, code, quotes, and links. Decisions and lookups stay on collapsed disclosure rows. A `◉ brain` mark on an answer opens the memories that were in its prompt. A new investigation stays untitled until the Recon model names it from the first question. `:rename` replaces that title.

Brain, OSINT, Providers, and System use fields and buttons only. OSINT keeps previous and next controls for saved manual runs. System shows hardware, paths, and a scrollable event log of run stages and failures.

Recon keeps threads, drafts, messages, run stages, plans, tool calls, evidence, and the memories supplied to each synthesis answer. Use Cancel to stop a run; interrupted runs can be resumed. The composer accepts `:rename <title>`, `:delete`, and `:delete-with-insights` for the selected thread.

The first answer in a thread is a broad reconnaissance pass. Recon selects three to five of the best suited tools, reports the names, domains, roles, and locations the evidence actually supports, and asks which narrower scope to pursue next: people, domains, emails, social accounts, infrastructure, or filings. Later turns go deeper on that scope and may use the configured call budget plus follow-up rounds. A broad question (`who`, `what`, `where`, `when`, `how`, or `why`, followed by `is`, `did`, `are`, and the same kind of verb) still checks Brain first. When those memories are missing or thin, Recon searches the web with Firecrawl before planning. Enter the key on the Firecrawl search tool in OSINT, or export `FIRECRAWL_API_KEY`. The call is `POST https://api.firecrawl.dev/v2/search` with `query` and `limit`. After those searches, Recon isolates up to three catalog tools that fit the question, such as SociaVault, Keybase, or Stack Exchange for a social-media question, and runs them with the subject's name, evidenced handle, or own domain. A tool that lacks a key or a usable input is skipped, and the decision block lists it under Tool isolation with the reason. Extracted entities are the subject and related people or organizations; the news sites that published a result are cited, not extracted.

Hunter domain search, email finder, email verifier, and company tech lookup share one key. Enter it on any Hunter tool, or export `HUNTER_API_KEY`. A saved key overrides the environment variable. SociaVault profile lookups use the same pattern (`SOCIAVAULT_API_KEY` on the SociaVault profile tool). On a later turn, handles and company domains found in Firecrawl results are looked up and logged in the transcript: SociaVault profiles the handles, and Hunter domain search plus tech lookup run for company domains. Social-network hosts such as `x.com` stay on the profile lookup. The opening turn still does not start that narrower work.

OSINT lists 36 HTTP lookup tools in thirteen categories, including Firecrawl search, Hunter enrichment, and SociaVault profiles. Choose a tool to see its input schema, example, documentation, access restrictions, result, and source. Manual runs remain in history and can be attached to a Recon thread without another request. Prev and Next move through saved manual results. A standalone result does not create a Brain insight; Recon extracts insights from cited evidence after synthesis. Some public services require an identifying User-Agent and impose quotas or licensing limits. Set one before SEC or Nominatim lookups:

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
