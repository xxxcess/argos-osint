# Argos OSINT

Argos is a terminal research desk. It keeps the Grok Build shape that matters
for a long session: a full-screen TUI, a dashboard you can leave and come back
to, and a prompt that always talks to whatever is on the canvas. The work it
does is different. A turn searches public sources, recalls what Argos knows
about you, and writes a markdown report.

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
| `config.toml` | layout, SearXNG URL, report directory, text/voice modality |
| `auth.json` | provider keys and the Gmail app password, mode `0600` |
| `argos.db` | cases, transcripts, brain, report index |
| `hardware.json` | cached host profile |
| `mcp.json` | Gmail MCP client config written from the Gmail app |

Reports default to `./reports` in the directory you launched from.

## The shell

The bottom composer is the same idea as Grok Build. Enter sends. The message
goes to the session bound to the open view: the desk on the dashboard, the
highlighted case on the case desk, or the module session for Hardware,
Providers, Brain, Gmail, Reports, the log, or Settings. Esc leaves the open
app and returns to the dashboard. Esc does not cancel a running turn.
Ctrl+C cancels a turn, clears a draft, or quits when the prompt is empty.

| Key | Action |
| --- | --- |
| Enter | Send the prompt to the view on the canvas |
| Tab | Jump focus: launcher, canvas, prompt |
| Ctrl+P | Search and launch an app |
| Ctrl+L | Next layout |
| Esc | Close the app search, or leave the open app |
| Ctrl+C | Cancel the turn, clear the draft, or quit |
| Ctrl+R | Record a few seconds and transcribe (voice) |
| Ctrl+U | Clear the prompt |
| Up / Down | Prompt history, or the slash menu |
| ? | Help, when the prompt is not focused |

`/search`, `/new`, `/use`, `/report`, `/hardware`, `/provider`, `/brain`,
`/gmail`, `/voice`, `/text`, `/layout`, `/open`, `/dashboard`, `/clear`, `/quit`.

`/use` matches a case by exact id or title, then by one unambiguous prefix.

## Layouts

Ctrl+L cycles the ten layouts from the reference sheet. The prompt stays put
in all of them. See [docs/layouts.md](docs/layouts.md).

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

## Providers

Text and voice only. Both speak the OpenAI-compatible HTTP API, which covers
xAI, OpenAI, Ollama, llama.cpp, and LM Studio.

* **local** — base URL, model, optional key. A typical Ollama URL is `http://127.0.0.1:11434/v1`.
* **api** — base URL, model, and a key typed without echo (`argos login`) or into the masked Providers field.
* **device** — RFC 8628. You supply the client id, device-authorization URL, and token URL. Argos prints the verification URL and the user code, then polls. It does not embed another product's OAuth client id.

Voice uses `POST {base}/audio/transcriptions`. Ctrl+R runs `rec` (sox) or
`ffmpeg` for five seconds. The transcript lands in the prompt so you can edit
it before Enter.

`argos login` is the same flow without the TUI. Details are in
[docs/providers-and-gmail.md](docs/providers-and-gmail.md).

## Hardware

The host profile reports OS, architecture, CPU name, logical cores, RAM, disk,
and a GPU when one is visible. Apple Silicon has no discrete VRAM. Argos
reports a Metal working set: 67% of RAM up to 16 GB, 75% up to 64 GB, 80%
above that, unless `iogpu.wired_limit_mb` is set. NVIDIA uses `nvidia-smi`.
The probe is cached for 30 minutes. `r` on the Hardware canvas rescans.

## Brain

`/brain the night desk prefers cited reports` or `remember …` stores a fact.
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
* Odysseus `agros`: `services/hwfit/hardware.py`, `services/memory`, `src/agent_loop.py`, `src/memory.py`, `src/copilot.py` (device-code shape), `docs/setup.md` (IMAP app passwords)
* [RFC 8628](https://www.rfc-editor.org/rfc/rfc8628) device authorization grant
* [MCP stdio transport](https://modelcontextprotocol.io/specification/2025-06-18/basic/transports) (newline-delimited JSON-RPC)
* [SearXNG search API](https://docs.searxng.org/dev/search_api.html)

Argos is original code. It follows those behaviors. It is not a fork of Grok
Build or Odysseus. This tree is Apache-2.0. Odysseus itself is AGPL-3.0.
