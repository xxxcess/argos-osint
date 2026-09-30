# Argos OSINT

Argos is a terminal research desk. It keeps the Grok Build shape that matters
for a long session: a full-screen TUI, a launcher, and a prompt fixed to the
bottom. The main canvas is the Case Desk. Ask about reviewed case evidence
and saved reports, open cited sources, and explicitly investigate an evidence
gap. A case can have leads and a network before any report exists. System and
Providers remain beside it.

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
Ctrl+C cancels a turn or clears a draft. Background jobs continue; `/cancel-jobs`
cancels them. With no active turn, draft, or jobs, Ctrl+C quits.

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

## Case Desk and investigations

See [case flow, configuration, migration, and limitations](docs/case-desk-research.md).
Ordinary questions retrieve reviewed observations and indexed report passages in
scope and show attributed excerpts with citations, recommendations, and unresolved
questions. They make no provider or model calls. The default general Desk searches
the saved collection; `/scope case <id>`, `/scope reports <ids>`, and `/scope report`
restrict retrieval. `/scope desk` selects legacy unassigned material.

`+` or `/new <question>` opens a scope card. Choose a new or existing case (`c`),
include existing recommended passages (`e`), and allow sensitive (`s`) or active
HTTP (`a`) actions if needed. Provider privacy and exact-host controls still apply.
First actions start unchecked; Space selects sources and Enter saves the case and
starts only selected actions. Enter with no actions creates a case without a report
or initial network. Selecting saved cases opens Inbox + Workbench. Esc restores the Desk prompt,
selection, and scroll position.

The Desk shows **Next Work**, a **Queue** including cases without reports, and typed
**Open Questions**. Focus these lists with `w/q/g`, move with `j/k`, and press Enter
to open the selected lead. `\` toggles the retrieval transcript; ordinary questions
remain retrieval-only.

The four modes are **1 Desk**, **2 Inbox + Workbench**, **3 Graph**, and **4 Product**.
The persistent inbox ranks pending review and typed gaps before accepted link degree,
showing a why-now reason and separate counts. Tab cycles inbox, center, inspector,
and composer; narrow terminals stack the inbox above the center.

Workbench keeps Plan, selected-lead jobs, Intake, and So What beside the inbox.
`e` focuses Plan; Space checks actions; `e` or Enter queues only the checked set as
separate bounded jobs. Checks start empty. `/investigate` remains a single explicit
action. Selecting a lead never collects. Discovered identifiers become candidate
leads without recursive collection. `a/r/d/t` prepares a review with a reason;
`s` cycles intake presentations and `J` opens selected-lead jobs (`/jobs` is case-wide).

Graph shows accepted edges, dashed candidates, and typed sockets: uncollected,
collected-absent, conflicting, and candidate. `g` selects holes; Enter on an
uncollected hole opens its checked plan without running it. Collected-absent opens
history and is not a real-world negative finding. `←/→` selects a link, `x` expands
one neighbor, `z` collapses, `o` opens source, and `v` opens review. `p` overlays Path;
`f/t` pins endpoints, `n/N` selects a path, and `[ / ]` selects a hop. Pins survive
center switches. Path uses accepted noncandidate links only and never researches.
No recovered path is not proof of a real-world gap.

Product lists eligible accepted observations; Space selects IDs, `c` toggles
lead/case scope, and Enter prefills `/draft final <IDs>`. Pending gaps stay unresolved.
`/work [lead]`, `/graph`, and `/gaps [lead|case]` expose the same cached surfaces.
Migration aliases `5` Review, `6` Jobs, and `7` Path remain available.

`D` opens case data controls. `/clear-case [id or title]` removes case investigation
data while keeping the empty case; `/delete-case [id or title]` also removes the
case. Both show counts and require `/confirm-case <exact ID>`; cancel with Esc or
`/cancel-case-data`. Active case jobs must finish or be cancelled first. Chat,
case-only evidence, reviews, scope/correction/identity history, jobs/cache, and the
case network are removed atomically. Saved reports and report-owned evidence are
retained and detached; historical versions/citations and other cases stay accessible.
This cannot be undone. Stale jobs cannot restore cleared data.

`/filter <text/source/state>` filters evidence; `s` cycles entity, relationship, source, event/retrieval date, and review-state sorts.

Review keys `a/r/d/t` prepare accept/reject/defer/retain; add a reason and Enter.
`/review <observation-id> accept|retain|reject|defer <reason>` keeps history. Accepted
observed links enter the case network; rejected/deferred evidence cannot promote
links. Co-occurrence and candidate identity remain candidates even after review.
Evidence count, acceptance, and network degree are separate. `/jobs` and
`/cancel-jobs` expose progress and cancel queued/running work; partial successes survive.

Reports are explicit outputs: `/draft final|addendum|revision|followup <accepted IDs>`
writes selected reviewed case evidence, source dates, uncertainties, and pending gaps.
It leaves case observations independent of report ownership. Historical reports,
versions, and `report-id@vN:Lline` citations remain readable using `/cite`. `/source
<observation-id>` opens an observation artifact. Existing report workspaces retain
`/save-update addendum|revision|followup` and reversible corrections/identity decisions.

Providers → Research contains **Discovery**, **Infrastructure**, **Identity &
contacts**, **Exposure**, and **Analysis & output** tabs (`1–5`). Cards expose purpose,
input types, mode, readiness, versions, credentials, limits, scope, cache/retries,
and Test Configuration. Models and account routing remain separate. Katana, Maigret,
and Mosint collection stays visibly unavailable until output/scope contracts are
verified. Opening configuration never scans or installs. Managed install/update/remove
show a plan before Apply starts a background job. No broad scan or paid lookup is
part of the test suite.

Migration is additive. Reports are indexed without changing their case association.
Assigned reports or explicitly included passages can propose case mentions and
co-occurrence for review. Old snapshots and citations remain available; credentials,
transcripts, raw observations, and decisions are retained. Interrupted research jobs
become partial; completed paid requests are not automatically replayed on restart.

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

The launcher is Case Desk, Providers, and System. Case Desk opens case investigations and historical reports alongside Brain. Providers keeps separate model accounts, Models, Sources, and its existing Research page. Mail and MCP setup remain separate. System has Log, Hardware, and Settings.

OSINT, under Providers, turns Facts, Web, News, Domain, Social, and Identity on or off for every run, sets a SearXNG URL, and can store Brave, Tavily, YouTube, and GitHub keys. Extra public sources whose URL contains `{query}` still work. Private addresses are refused.

On the Case Desk, ask saved evidence or use **+** to review an investigation
scope. Next Work opens the highest-priority lead; the case frame keeps an inbox
beside Workbench, Graph, or Product. Historical report workspaces retain their five
layout shortcuts (`g/q/p/m/r`), citation reader, and extraction audit.
Case evidence and typed relationships remain the source of truth for investigation TNA.

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

Report discussion is temporary. `/retain-answer` explicitly saves a completed answer as a report-tagged fact; ordinary Desk retrieval does not silently file facts. Retained facts survive restart. Deleting a report preserves the existing confirmation flow and removes its report-tagged facts.
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
