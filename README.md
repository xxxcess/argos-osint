# Argos OSINT

Argos is a terminal research desk. It keeps the Grok Build shape that matters
for a long session: a full-screen TUI, a launcher, and a prompt fixed to the
bottom. The main canvas is the Case Desk. Ask it about completed reports,
open a cited passage, inspect its report network, and explicitly start
focused research when the saved evidence has a gap. System and Providers
remain beside it.

There is no browser UI. Every screen is the terminal.

The Odysseus reference for hardware profiling, the user-centric agent loop,
provider login, brain recall, and Gmail is the `agros` branch. That repository
has no branch named `argos`.

## Run

```sh
cargo run -p argos-osint-bin
```

The binary is `argos` (`target/debug/argos`).

```sh
argos                  # open the terminal
argos login            # text or voice provider, from the terminal
argos logout           # forget text and voice credentials
argos logout --gmail   # also forget the Gmail app password
argos hardware         # print cores, RAM, VRAM, architecture
argos reports          # list markdown reports
argos -p "search public reporting on the port authority"
argos mcp gmail        # Gmail-only MCP server on stdio
```

State lives in `~/.argos` (`ARGOS_HOME` overrides it):

| Path | What |
| --- | --- |
| `config.toml` | SearXNG URL, source stages, research tool settings, report directory, text/voice modality |
| `auth.json` | model and research credentials and the Gmail app password, mode `0600` |
| `argos.db` | cases, transcripts, passage index, research jobs, evidence, and graph snapshots |
| `hardware.json` | cached host profile |
| `mcp.json` | optional legacy Gmail MCP client config; Providers does not create it |

Reports default to `./reports` in the directory you launched from. Each file is a BLUF product. The bottom line is a short summary of every public source that answers the requirement, then the requirement, low-confidence source judgments, evidence with the URL and retrieval time, gaps, and a source list. The summary does not add claims beyond those excerpts.

## The shell

The bottom composer is the same idea as Grok Build. Enter sends. The message
goes to the case desk, or to the case you selected with J/K, Enter, or
`/use`. Esc closes an open widget first, then points the prompt back at the
case desk. Esc does not cancel a running turn.
Ctrl+C cancels a turn, clears a draft, or quits when the prompt is empty.

| Key | Action |
| --- | --- |
| Enter | Send the prompt to the case desk or the selected case |
| Tab | Jump focus: launcher, canvas, prompt |
| Ctrl+P | Search and launch an app |
| Esc | Close the app search, or leave the open app |
| Ctrl+C | Cancel the turn, clear the draft, or quit |
| Ctrl+R | Record a few seconds and transcribe (voice) |
| Ctrl+U | Clear the prompt |
| Up / Down | Prompt history, or the slash menu |
| Click the chat, or Tab to it | Focus the case desk or report chat. Up and Down, and the scroll wheel, then move through that history. End returns to the latest line |
| ? | Help, when the prompt is not focused |

`/search`, `/new`, `/use`, `/report`, `/hardware`, `/provider`, `/brain`,
`/gmail`, `/voice`, `/text`, `/open`, `/dashboard`, `/clear`, `/quit`.

## Case Desk and report evidence

See [configuration, migration, and implementation status](docs/case-desk-research.md)
for the supported execution paths and remaining work.

Ordinary Desk questions search completed report passages first. Matching
passages are ranked by text relevance and shown with report dates; answers
cite `report-id@vN:Lline`. `/cite 1` opens the first recommended passage,
and `/cite report-id@vN:Lline` opens a saved citation, including earlier
report versions. If no passage supports a claim, the Desk reports the gap
instead of creating an investigation. `/new <question>` opens a scope card;
an explicit fresh-research request also opens that card and carries any
relevant saved passages into the turn. Use `/scope report`, `/scope reports
<ids>`, `/scope case <id>`, or `/scope collection` to expand the default
report/case boundary deliberately.

In a report, `R` or `/read` opens its text, `j/k` moves through lines, and
`g` explores the current report graph from the cited passage. `/related`
retrieves related passages and `/entities` opens the graph. Report questions
are evidence-only by default and do not file facts or new reports;
`/retain-answer` explicitly keeps a completed answer. Historic citations
remain readable after revisions; the graph always reflects the latest
report text.

The Research tab under Providers is separate from model accounts. It exposes
provider readiness, execution mode, scope and privacy controls, limits,
masked credentials, and Test Configuration. Existing search, domain,
InternetDB, GitHub identity, and LeakCheck paths remain available. Shodan,
XposedOrNot, and selected WhatsMyName checking have structured adapters;
credentials, provider entitlements, and privacy settings may still be
required. Katana has an explicit pinned, checksum-verified managed installer.
SpiderFoot, Mosint, and Maigret appear as configured capabilities but their
collection paths remain unavailable until an installed version's output and
scope contract can be verified. Opening Providers never installs tools or
starts a scan.

`/investigate <provider> [input]` submits a bounded background enrichment
job; `/jobs` shows job state and `/cancel-jobs` cancels queued/running work.
In a report network, `i` opens the cached evidence inspector. `/findings`
shows observations; `/review <observation-id> accept|retain|reject|defer
<reason>` records an analyst decision. `/save-update addendum|revision|followup`
saves accepted observations with attribution while retaining earlier files,
versions, and citations. `/timeline` shows dated and undated observations
without treating retrieval time as event time. `/correct` and `/merge` have
reversal commands; original labels and mentions are retained.

On first run, existing report files are imported into a versioned SQLite
passage index. Legacy OSINT keys in `config.toml` move into `auth.json`.
The index and graph snapshots rebuild when reports or extraction rules
change. Research jobs interrupted by a restart are marked partial; Argos
does not replay completed paid requests automatically.

`/clear` wipes the chat on screen: the case desk, or the report chat when one is open.

`/use` matches a case by exact id or title, then by one unambiguous prefix.

Apps stays on the left. The main area is the case desk, or the app you opened
from Apps. The prompt stays at the bottom.

## What a turn does

1. Search completed report passages for ordinary questions and preserve
   case/report scope.
2. Answer from cited evidence, identify gaps, or open a research scope card
   when fresh investigation is explicitly requested.
3. Run selected public sources in bounded background jobs for investigations;
   keep partial results when a provider fails.
4. Review collected observations before promoting them into a report update.

Without a model provider, the Desk still lists supporting passages and
citations. Explicit `/search` and investigation turns still write a source
pack; the narrative waits until a model is signed in.

Public page fetches allow http and https on ports 80 and 443, and refuse
loopback, link-local, and private addresses. Argos does not open a general
shell, and it does not help with unauthorized access, credential theft, or
covert surveillance.

## Chat Providers

The launcher is Case Desk, Providers, and System. Case Desk keeps Desk, Brain, and the report network. The report list shows pending and completed reports; selecting a recommended report opens its supporting passage. Providers has separate model accounts, Models, Sources, and Research pages. Mail and MCP setup remain separate. System has Log, Hardware, and Settings.

OSINT, under Providers, turns Facts, Web, News, Domain, Social, and Identity on or off for every run, sets a SearXNG URL, and can store Brave, Tavily, YouTube, and GitHub keys. Extra public sources whose URL contains `{query}` still work. Private addresses are refused.

On the Case Desk, type a question to search existing reports, or press **+** to choose Facts, Web, News, Domain, Social, and Identity for a new run. Space toggles a source, Enter starts, and Esc cancels. Domain-specific sources skip registration and certificate lookups without a domain. **Tab** focuses the report list. **Enter** opens a relevant passage when one is recommended, or opens the selected report workspace. Report questions remain temporary until explicitly retained. **Esc** returns to the Desk with its prompt and scroll position. **J**/**K** move the report highlight. **x** deletes the selected case after its existing confirmation flow.

The report workspace has five layouts over the same saved graph: **g** Cockpit, **q** Clusters, **p** Path, **m** Matrix, and **r** Ribbon. **Left/Right** switch layouts, **/** opens entity Find, and **Tab** cycles workspace focus and Ask. Path exposes each hop's evidence; Matrix retains adjacency and adds theme-by-report coverage with **c**; Ribbon scrubs report text and accepted/rejected extraction. **i** opens the shared cached evidence inspector. Graph links derived from text proximity are labeled co-occurrence, never ownership or verified identity. **?** opens workspace help.

The agent loop supports independently selected Writer and Tools connections.
Grok and OpenRouter use the OpenAI-compatible HTTP API. OpenAI subscription
access uses Codex CLI for Writer. Voice keeps its independent transcription API
connection.

| Provider | Connection | Authentication |
| --- | --- | --- |
| `grok` (default) | Grok Build login / `https://api.x.ai/v1` | Grok subscription sign-in |
| OpenAI / ChatGPT | Codex CLI (Writer) | ChatGPT subscription sign-in |
| `openrouter` | `https://openrouter.ai/api/v1` | typed key, or `OPENROUTER_API_KEY` |
| `local` | `http://127.0.0.1:11434/v1` | optional (Ollama, llama.cpp, LM Studio) |

With nothing else saved, text starts on Grok at `grok-4.6`. If `grok login --oauth`
has already been run, Argos uses that subscription session from `~/.grok/auth.json`.
Saved Grok API keys and API-key environment variables are not used. `Ctrl+M`
or `/model` opens the picker (Grok 4.6 and Grok 4.5, plus whatever
`GET /v1/models` returns). `/model grok-4.5` switches.
`argos models` prints the list, and `argos -m grok-4.5` selects one for a
headless turn or saves it when you open the TUI.

Providers keeps each account independent. **Grok** offers subscription sign-in
only through Grok Build CLI on `PATH`. Select Sign in with Grok and complete browser sign-in, or run `grok login --oauth` and select Check existing login.
Choose Grok during `argos login` to open subscription sign-in. Grok can serve both Writer
and Tools; model access depends on the signed-in account's entitlements.
OpenRouter retains its masked API-key field and `OPENROUTER_API_KEY` support.
Keys are stored owner-only in `auth.json`; switching providers or models never
replaces another account's credentials. Existing Grok keys remain stored for
compatibility but are ignored by the subscription connection.

**OpenAI** offers **ChatGPT subscription sign-in only**, through a recent Codex
CLI on `PATH`. Select Sign in with ChatGPT and follow the device URL/code, or
run `codex login` and select Check existing login. Codex keeps its own credentials.
Argos does not request an OpenAI API key or fall back to API billing.

The separate **Models** page has a **Writer** card for answers/reports and a
**Tools** card for research. Enter on Provider opens an account picker; Enter on
Model opens that account's catalog. You can search or enter a model ID directly;
F5 refreshes API catalogs. Each role saves its provider and model independently.
ChatGPT subscription is Writer-only; Tools uses Grok or OpenRouter (existing
local connections remain available). Subscription answers arrive as complete
Codex message chunks, rather than API token deltas. The Codex process uses an
isolated temporary directory, read-only permissions, disabled research tools,
and an ephemeral session.

Verify connection tests the current account form without saving it. Verified
drafts say Save to use. Save commits only that account. Choosing
`openrouter/free` opens concrete free models so you can pin a model instead of
the router's random selection. `Ctrl+M`, `/model`, and `-m` select the Writer.
The legacy `argos login` command also retains other saved API accounts; choosing
OpenAI starts ChatGPT sign-in. OpenRouter keeps its attribution headers.

Voice posts the recording to `{base}/audio/transcriptions`. Ctrl+R runs
`rec` (sox) or `ffmpeg` for five seconds. The transcript lands in the prompt
so you can edit it before Enter.

Details are in [docs/providers-and-gmail.md](docs/providers-and-gmail.md).

## Hardware

The host profile reports OS, architecture, CPU name, logical cores, RAM, disk,
and a GPU when one is visible. Apple Silicon has no discrete VRAM. Argos
reports a Metal working set: 67% of RAM up to 16 GB, 75% up to 64 GB, 80%
above that, unless `iogpu.wired_limit_mb` is set. NVIDIA uses `nvidia-smi`.
The probe is cached for 30 minutes. `r` on the System hardware tab rescans. `/hardware`, `/log`, and `/settings` open those System tabs.

## Brain

Each completed answer in an open-report network is paraphrased into a short fact and tagged with that report. Failed or cancelled answers do not file facts. Questions and answers are temporary; the distilled facts survive restart. Brain only shows those facts. Deleting a report asks again, then removes its markdown file and the facts taken from it.
The next turn scores memories by token overlap, with a boost for identity
notes ("my name is …") when you ask who you are. The matches are injected as
`USER MEMORY` and shown in the stream as a recall note. API keys and the
Gmail app password are not injected.

## Gmail

The mailbox path is Gmail only: `imap.gmail.com:993`, an app password, read
only. The Gmail app can save the account, count INBOX, and write
`~/.argos/mcp.json`. `argos mcp gmail` speaks newline-delimited JSON-RPC
(and Content-Length frames) with `gmail_list_recent` and `gmail_search`.
Sending mail is not implemented.

## Search endpoint

Set `searx_url` on the System settings tab or in `~/.argos/config.toml` to a SearXNG base URL.
Argos calls `GET /search?q=&format=json`, and `categories=news` for the news stage.
JSON output has to be enabled on that instance. An empty URL falls back to the
DuckDuckGo instant-answer JSON API. Brave and Tavily run as well when
`brave_key` or `tavily_key` is set (or `BRAVE_API_KEY` / `TAVILY_API_KEY`).
Facts use the Wikipedia and Wikidata JSON APIs. News also uses GDELT. A domain
in the question adds RDAP, crt.sh, DNS-over-HTTPS, Wayback, and InternetDB.
A topic or handle adds Bluesky, Hacker News, Mastodon, and YouTube when
`youtube_key` or `YOUTUBE_API_KEY` is set. A handle or email adds GitHub.

## Tests

```sh
cargo test --manifest-path Cargo.toml
```

Run that from this directory so rustup picks the pinned toolchain in
`rust-toolchain.toml` (Rust 1.94).

## Docs read while building this

* Grok Build user guide in `grok-build` (authentication, keyboard shortcuts, dashboard, getting started) and [docs.x.ai/build/overview](https://docs.x.ai/build/overview)
* Odysseus `agros`: `services/hwfit/hardware.py`, `services/memory`, `src/agent_loop.py`, `src/memory.py`, `src/llm_core.py` (provider host detection and OpenRouter headers), `docs/setup.md` (IMAP app passwords)
* [MCP stdio transport](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports) (newline-delimited JSON-RPC)
* [SearXNG search API](https://docs.searxng.org/dev/search_api.html)

Argos is original code. It follows those behaviors. It is not a fork of Grok
Build or Odysseus. This tree is Apache-2.0. Odysseus itself is AGPL-3.0.
