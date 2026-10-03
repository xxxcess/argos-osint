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

Recon keeps threads, drafts, messages, run stages, plans, tool calls, evidence, and the memories supplied to each synthesis answer. Use Cancel to stop a run; interrupted runs can be resumed. The composer accepts `:rename <title>`, `:delete`, and `:delete-with-insights`. Both delete commands remove the investigation and the Brain memories that belong to it.

Each turn checks Brain first. Recon then turns the prompt into one to five directives (d1 onward), inferring what each one should establish from the prompt. A goal is a short imperative such as "Find the subject's official online accounts and websites", with the entities it is about (taken verbatim from your prompt, or from the thread's subject on a follow-up like "what about his companies?") and the kinds of values it targets. Directives never name a tool or provider; if Recon's reply breaks a rule twice, a fixed set is used. The **Tool picker** model builds the tool order one pick at a time: each request offers the remaining eligible tools and returns the single best next tool, until the list reaches the call budget (at most thirteen tools), the picker says `done` (offered only after three picks), or no tools remain. The first pick comes from the primary providers (Firecrawl, SociaVault, Hunter), and gap-filler tools join after it. Tools that need another tool's output (for example SociaVault needs a handle from a search) are moved after their producer. Recon runs the list one call at a time. Inputs that can take the subject (a Wikidata name, a search query) get the subject; other inputs come only from values found in earlier results, each tied to its evidence id. Every input records where it came from, and a step with an input that can't be traced to a directive entity, an accepted value, or a fixed qualifier is skipped. Search queries are short: the entity, plus at most one fixed qualifier (`official account`, `official website`, `company`, `contact`), so an accounts directive searches `Elon Musk official account` and a company directive searches `Elon Musk company`, never the question text. A domain, organization, email, or URL taken from a search result counts only when that result mentions the subject, so unrelated SEO pages add nothing. Handles come from profile links and @mentions next to a platform name, and only accounts that belong to the subject count. SociaVault runs once per platform the questions ask about, within a per-turn SociaVault credit budget (3 credits on a thread's first turn and 8 later, never more than the credits left). A handle borrowed from another platform is marked inferred. When a Firecrawl search is weak (it failed, or returned fewer than three results, or only social and publisher pages), Recon adds one SociaVault Google search with the same query (a Google search is never given a new query). Hunter takes inputs only from your question, Firecrawl, SociaVault, or an earlier Hunter call. A value that only a gap-filler such as crt.sh found waits until a primary provider also reports it. If a call fails or a later step still lacks an input, Recon asks the picker for one replacement, for example an accounts search (`<subject> official account`) when no handle was found. A handle a directive names counts as an unverified input. Steps whose inputs are known still run when another step fails, and steps that cannot be filled are skipped with the reason; the turn still answers from the evidence it has. Q&A, wiki, and aggregator sites such as Quora or Reddit are cited, never treated as the subject's site or name. The answer addresses your question, then says for each directive (D1 onward) whether it was met, partly met, or not met, with citations (one evidence id per bracket; a bracket listing several ids is accepted and stored as `[a][b]`, and an unknown id is dropped after one repair as long as a valid citation remains), and asks you to narrow the scope only when a directive is not met. The decision row lists the directives, the picker and its model, the ordered tools with the directives they serve and their dependencies, the inputs bound for each step, and any fallback requests. If the picker is unconfigured, unreachable, or rate limited, a deterministic order is used and the turn still completes. News sites and account platforms are cited or attached as handles, not extracted as entities.

All nine Hunter tools share one key. Enter it on any Hunter tool, or export `HUNTER_API_KEY`. A saved key overrides the environment variable. The SociaVault tools work the same way with `SOCIAVAULT_API_KEY`, and the Firecrawl tools with `FIRECRAWL_API_KEY`. The old `hunter_tech_lookup` id still runs as `hunter_company_enrichment`. Handles and company domains found in earlier results feed SociaVault profiles and Hunter domain lookups in the same turn when the picker orders them. Social-network hosts such as `x.com` stay on the profile lookup and are never sent to Hunter.

OSINT lists 55 HTTP lookup tools in fifteen categories. Of these, 20 belong to the primary providers:

| Provider | Tools | Routes |
|---|---|---|
| Firecrawl | `firecrawl_search`, `firecrawl_scrape`, `firecrawl_map`, `firecrawl_batch_scrape`, `firecrawl_crawl` (off by default), `firecrawl_extract` | 6 endpoints |
| SociaVault | `sociavault_profile` (10), `sociavault_search` (12), `sociavault_search_users` (3), `sociavault_user_content` (18), `sociavault_google_search` (1) | 44 one-credit routes; no followers/following or single-post routes |
| Hunter | `hunter_domain_finder`, `hunter_email_count`, `hunter_domain_search`, `hunter_email_finder`, `hunter_email_verifier`, `hunter_company_enrichment`, `hunter_email_insight`, `hunter_person_enrichment`, `hunter_combined_enrichment` | 9 read endpoints |

Five more are keyed context tools (issue #29):

| Provider | Tools | Key |
|---|---|---|
| NewsAPI (News) | `newsapi_search` (`/v2/everything`), `newsapi_headlines` (`/v2/top-headlines`) | `NEWSAPI_API_KEY` or the key field on a News tool; sent as `X-Api-Key` |
| CourtListener (Legal) | `courtlistener_case_search` (opinions), `courtlistener_docket_search` (federal dockets), `courtlistener_judge_search` | `COURTLISTENER_API_TOKEN` or the key field on a Legal tool; sent as `Authorization: Token …` |

Recon uses them only when the prompt asks about news, current events, recent activity, or controversies (`news`), or about lawsuits, court cases, litigation, rulings, judges, or legal trouble (`legal`); a plain "who is X?" uses neither. They search the bare subject as an exact phrase (`"Elon Musk"`), take dates only when the prompt writes one ("since March 2026"), read one page (at most 10 NewsAPI articles or 20 CourtListener results), and keep only results whose title or snippet names the subject. At most 2 NewsAPI and 3 CourtListener calls run per turn (`recon limits --news-calls-per-turn`, `--legal-calls-per-turn`), CourtListener requests are spaced 12 s apart, and a CourtListener 429 skips the rest of that turn's CourtListener steps. Free-tier NewsAPI articles are 24 hours old and reach back one month. A tool without its key shows "needs key" and is never picked. Their results are evidence and citations only; they feed no other tool.

The other 30 tools are gap-fillers (DNS, certificates, RDAP, archives, code search, registries, geocoding, wallets, CVEs, IP reputation). Choose a tool to see its input schema, example, documentation, access restrictions, result, and source. Manual runs remain in history and can be attached to a Recon thread without another request. Prev and Next move through saved manual results. A standalone result does not create a Brain insight; Recon extracts insights from cited evidence after synthesis. Some public services require an identifying User-Agent and impose quotas or licensing limits. Set one before SEC or Nominatim lookups:

```sh
cargo run -p argos-osint-bin -- osint user-agent 'Argos contact@example.com'
```

Brain retains manual save, recall, pin, edit, and delete. Investigation insights have entity and topic anchors, evidence sources, and merged provenance. Providers keeps Grok, OpenAI, and OpenRouter connections, plus independent **Recon**, **Tool picker**, and **Synthesis** choices under **Defaults** (a three-way selector; the Tool picker defaults to OpenRouter `typesafe/jev-1.13`, which uses the OpenRouter decisions API). Account sign-in does not change any default. System shows hardware and storage paths.

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
argos defaults set tool-picker --provider openrouter --model typesafe/jev-1.13
argos defaults set synthesis --provider openrouter --model <model-id>
argos models --role recon
argos models --role tool-picker
argos insights --entity example.org
argos remember --app research --conversation thread-123 'A manually saved fact'
argos recall 'What do I know?'
```

`recon show` includes each run's plan, with the directives, input grounding, bindings, and per-pick tool-picker records (transport, confidence, reason). `recon delete` removes the thread and its contribution links. The default keeps Brain memories and marks their source deleted. `--with-insights` removes every Brain memory that belongs only to that investigation, including pinned and edited ones, and removes each memory's saved recon-path summary. A memory still sourced by another investigation stays. `recon limits` shows or changes the per-turn round, call, and time budgets, including `--max-turn-seconds` (default 900, range 120 to 1800), the hard ceiling for one turn. A turn's deadline grows with recon rounds, scheduled tool time, and the evidence packet, and it never drops below `turn_seconds`. Synthesis text streams into the transcript and to stderr for `argos ask`; stdout stays the final JSON. `osint enable` and `osint disable` control tools. CLI commands return JSON where practical and use the same store, registry, and executor as the TUI.

## State and migration

State lives in `~/.argos`; `ARGOS_HOME` overrides the directory. `argos.db` holds Brain memories, threads, runs, calls, bounded cached responses, entities, and provenance. `config.toml` holds role defaults and OSINT settings. `auth.json` holds provider credentials with owner only permissions on Unix; `hardware.json` caches the host profile.

Opening the database performs additive versioned migrations. Existing Brain memories and unrelated tables are preserved. Old Writer settings seed the Recon and Synthesis defaults once; the Tool picker default is seeded only when it is empty. Subsequent loads preserve every role choice. A run interrupted by process exit keeps completed observations and is marked interrupted on the next TUI launch. Resume explicitly continues remaining steps; retry starts a new turn.

## Limits and verification

The tool adapters issue bounded public HTTP requests, but public upstream availability and quotas can change. A registry entry or fixture test does not establish current live availability. Responses are observations with retrieval times, not proof of current ownership, personal identity, or exploitability. Shodan InternetDB's free access is restricted to noncommercial use. Public Nominatim requires an identifying User-Agent, attribution, caching, and a maximum of one request per second. Provider model calls use the account and terms you configure. [Architecture](docs/architecture.md) and [provider setup](docs/providers.md) give more detail.

```sh
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
