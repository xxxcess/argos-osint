# Argos OSINT

Argos is a terminal research desk. It keeps the Grok Build shape that matters
for a long session: a full-screen TUI, a launcher, and a prompt fixed to the
bottom. The main canvas is the case desk. The prompt talks to that desk, or
to the one case you have selected. System, providers, and brain open
beside it. A turn searches
public sources, recalls what Argos knows about you, and writes a markdown
report.

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
| `config.toml` | SearXNG URL, source stages, API keys, report directory, text/voice modality |
| `auth.json` | provider keys and the Gmail app password, mode `0600` |
| `argos.db` | cases, transcripts, brain, report index |
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

`/clear` wipes the chat on screen: the case desk, or the report chat when one is open.

`/use` matches a case by exact id or title, then by one unambiguous prefix.

Apps stays on the left. The main area is the case desk, or the app you opened
from Apps. The prompt stays at the bottom.

## What a turn does

1. Classify the message (investigate, remember, hardware, gmail, chat).
2. Recall a few brain notes and inject them into the system prompt, along with
   the open view and a one-line host profile.
3. For an investigation, research the enabled public stages before the model
   speaks, so a local model that cannot call tools still sees the hits.
4. Ask the signed-in text provider, honoring tool calls for another web, news,
   domain, social, or identity lookup, a public page fetch, a memory, or a report.
5. Write `./reports/<id>-<slug>.md` with YAML front matter, findings, and sources.

Without a provider, `/search` and an investigate turn still write a source
pack. The narrative waits until a model is signed in.

Public page fetches allow http and https on ports 80 and 443, and refuse
loopback, link-local, and private addresses. Argos does not open a general
shell, and it does not help with unauthorized access, credential theft, or
covert surveillance.

## Chat Providers

The launcher is Case Desk, Providers, and System. Case Desk tabs are Desk and Brain. Brain replaces the desk and the report list. Brain lists fact memories and opens a card to read one. New facts come from completed answers in the open-report network workspace. The report list stays beside the desk and shows pending and completed reports. A failed task stays marked failed there; the reason is only in the System log. Providers has dedicated Grok, OpenAI ChatGPT, and OpenRouter account pages, a separate Models page for Writer / Tools assignments, and a Sources page for OSINT. Mail and MCP setup are not part of Providers. System tabs are Log, Hardware, and Settings. Left and right move between those pages. The log is timestamped and holds system calls, API calls, failed tasks, and searches. Settings is the SearXNG URL and the report folder.

OSINT, under Providers, turns Facts, Web, News, Domain, Social, and Identity on or off for every run, sets a SearXNG URL, and can store Brave, Tavily, YouTube, and GitHub keys. Extra public sources whose URL contains `{query}` still work. Private addresses are refused.

On the case desk, type a query and press **+** to choose Facts, Web, News, Domain, Social, and Identity for that run, then start a case worker. Space toggles a source, Enter starts, and Esc cancels. The defaults come from OSINT settings and the card does not change them. Domain tools still skip registration and certificate lookups when the query has no domain. **Tab** focuses the report list. **Enter** or a click asks you to confirm. Confirming opens that report’s network Cockpit. The report workspace hides Desk/Brain tabs and keeps the bottom composer for evidence-only questions. Answers stream into a temporary popup; questions and answers are never stored as chat messages. Completed answers still produce report-tagged Brain facts. **Esc** clears Find, then dismisses the answer, then closes the report and restores Desk. A normal desk message is checked against fact memories from completed reports first. A hit is answered from those facts, and the reply names the reports to open. If nothing matches, or the question asks for a new case anyway, a confirmation offers to just answer or start a case worker. Starting a case worker opens the same source card before research begins. The report list shows that task as pending, failed, or completed. **J**/**K** move the highlight. **x** deletes that case.

The report workspace has five layouts over the same saved graph: **g** Cockpit (entities, ego boxes, link ledger), **q** Clusters (four type regions, degree-ranked anchors, missing co-occurrence links), **p** Path (FROM/TO pins and up to five paths within four hops), **m** Matrix (group counts and up to 24 entities), and **r** Ribbon (report-order evidence and extraction decisions). **Left/Right** switch layouts, **/** opens entity Find, **Enter** finishes Find, and **Tab** cycles workspace focus and Ask. In Cockpit, **[ / ]** cycle links and detail **j/k** pages ego boxes. In Path, **f/t** set pins and detail **j/k** selects paths. Matrix uses **hjkl** and **Enter** returns to Cockpit. Ribbon uses **h/l** to scrub report lines and **d** to show rejected candidates. **?** opens workspace help. Research commands and Brain editing are Desk-only.

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
