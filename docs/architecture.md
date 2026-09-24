# Architecture

Two crates.

`argos-osint-core` is the desk: cases, SQLite, brain recall, hardware probe,
provider login and chat, public search, markdown reports, and the Gmail MCP
codec. It does not know about the terminal. Provider login is a catalog of
Grok, OpenAI, OpenRouter, and local. The turn runner only sees an
OpenAI-compatible base URL, a resolved bearer token, and optional headers.

`argos-osint-bin` is the `argos` binary. `cli` handles login, logout,
hardware, reports, headless `-p`, and `mcp gmail`. `tui` is the screen.

A turn is built in `agent::run_turn`. The TUI owns the transcript and the
database. The core returns events: status, streamed text, notes, memories,
reports, the final answer, or a failure. The open view is a string the TUI
fills in (`Dashboard`, `Case · …`, `Hardware`, …) plus a short description of
what is on the canvas. That pair is the system prompt's `ACTIVE VIEW`, so the
same composer can talk to whichever module is showing.

Sessions have stable ids. `desk` is the dashboard. `module:hardware` and the
other module ids are the setup apps. A case id from `/new` is the
investigation. Switching the canvas switches which transcript the prompt
appends to.

`Store::call_states` exists only in tests. The tool-call table is written in
every build; the inspector is not part of the library surface.

Hardware detection shells out the same way a host probe has to: `sysctl` and
`system_profiler` on Apple Silicon, `nvidia-smi` when it is on `PATH`, and
`sysinfo` for CPU, RAM, and disks. The Metal budget helper is pure and tested
without a Mac GPU.

Search prefers SearXNG's JSON API. The DuckDuckGo HTML parser is a fallback
and is tested against a fixture, not the live site. `fetch_page` checks the
host before the request and refuses private, loopback, and link-local
answers, including IPv4-mapped IPv6.
