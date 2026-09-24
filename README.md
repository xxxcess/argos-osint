# Argos OSINT

Argos is a terminal research desk. It keeps the Grok Build shape that matters
for a long session: a full-screen TUI, a launcher, and a prompt fixed to the
bottom. The main canvas is the case desk. The prompt talks to that desk, or
to the one case you have selected. Hardware, providers, brain, Gmail,
reports, the log, and settings open as widgets beside it. A turn searches
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
| `config.toml` | SearXNG URL, report directory, text/voice modality |
| `auth.json` | provider keys and the Gmail app password, mode `0600` |
| `argos.db` | cases, transcripts, brain, report index |
| `hardware.json` | cached host profile |
| `mcp.json` | Gmail MCP client config written from the Gmail app |

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
3. For an investigation, search before the model speaks, so a local model that
   cannot call tools still sees the hits.
4. Ask the signed-in text provider, honoring tool calls for another public
   search, a public page fetch, a memory, or a report.
5. Write `./reports/<id>-<slug>.md` with YAML front matter, findings, and sources.

Without a provider, `/search` and an investigate turn still write a source
pack. The narrative waits until a model is signed in.

Public page fetches allow http and https on ports 80 and 443, and refuse
loopback, link-local, and private addresses. Argos does not open a general
shell, and it does not help with unauthorized access, credential theft, or
covert surveillance.

## Chat Providers

The launcher is Case Desk, Providers, Hardware, Search Log, and Settings. The Brain tab replaces the desk and the report list. It lists fact memories and opens a card to read one. New facts come from report-chat replies. The report list stays beside the desk and shows pending and completed reports. Providers' side pages are Mail, OSINT, and LLM. Left and right move between those pages. Search Log stays its own app. Hardware stays its own app. Settings is the SearXNG URL and the report folder.

OSINT, under Providers, turns internet search and Wikipedia on or off, sets a SearXNG URL, and adds extra public sources whose URL contains `{query}`. Private addresses are refused.

On the case desk, type a query and press **+** to start a case worker immediately. **Tab** focuses the report list. **Enter** or a click asks you to confirm. Confirming replaces the case desk chat with that report's chat, including any earlier questions. **Esc** returns to the desk. Questions there are answered only from that report, and the reply says when the file does not cover them. **Esc** returns to the desk. A normal desk message is checked against fact memories from completed reports first. A hit is answered from those facts, and the reply names the reports to open. If nothing matches, or the question asks for a new case anyway, a confirmation offers to just answer or start a case worker. The report list shows that task as pending, failed, or completed. **J**/**K** move the highlight. **x** deletes that case.

The agent loop is vendor-neutral. Text and voice both speak the
OpenAI-compatible HTTP API. Login only selects who hosts that API. The text
slot and the voice slot are chosen separately, so chat can be Grok while
transcription is local.

| Provider | Base URL | Key |
| --- | --- | --- |
| `grok` (default) | `https://api.x.ai/v1` | typed key, or `XAI_API_KEY` |
| `openai` | `https://api.openai.com/v1` | typed key, or `OPENAI_API_KEY` |
| `openrouter` | `https://openrouter.ai/api/v1` | typed key, or `OPENROUTER_API_KEY` |
| `local` | `http://127.0.0.1:11434/v1` | optional (Ollama, llama.cpp, LM Studio) |

With nothing else saved, text starts on Grok at `grok-4.6`. If `grok login`
has already been run, Argos uses that session from `~/.grok/auth.json`. An
`XAI_API_KEY` or `argos login` key is used instead when one is set. `Ctrl+M`
or `/model` opens the picker (Grok 4.6 and Grok 4.5, plus whatever
`GET /v1/models` returns). `/model grok-4.5` switches.
`argos models` prints the list, and `argos -m grok-4.5` selects one for a
headless turn or saves it when you open the TUI.

`argos login` prints the four providers and asks which slot (text or voice) to fill.
A cloud key is read with echo off. If the vendor's environment variable is
already set, Argos can keep using it and leave `auth.json` without a copy.
OpenRouter requests also send `HTTP-Referer` and `X-Title`. The base URL
stays editable, so a proxy in front of the same provider still works.

On the Providers LLM page, Enter on the provider row cycles `grok`, `openai`,
`openrouter`, and `local`, and fills the default URL and model when you have
not customized them.

Voice posts the recording to `{base}/audio/transcriptions`. Ctrl+R runs
`rec` (sox) or `ffmpeg` for five seconds. The transcript lands in the prompt
so you can edit it before Enter.

Details are in [docs/providers-and-gmail.md](docs/providers-and-gmail.md).

## Hardware

The host profile reports OS, architecture, CPU name, logical cores, RAM, disk,
and a GPU when one is visible. Apple Silicon has no discrete VRAM. Argos
reports a Metal working set: 67% of RAM up to 16 GB, 75% up to 64 GB, 80%
above that, unless `iogpu.wired_limit_mb` is set. NVIDIA uses `nvidia-smi`.
The probe is cached for 30 minutes. `r` on the Hardware canvas rescans.

## Brain

Each finished reply in a report chat is paraphrased into a short fact and tagged with that report. Brain only shows those facts. Deleting a report asks again, then removes its markdown file and the facts taken from it.
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

Set `searx_url` in Settings or `~/.argos/config.toml` to a SearXNG base URL.
Argos calls `GET /search?q=&format=json`. JSON output has to be enabled on
that instance. An empty URL falls back to the public DuckDuckGo HTML results.

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
