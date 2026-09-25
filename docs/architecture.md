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

Sessions have stable ids. `desk` is the dashboard. `module:system` and the
other module ids are the setup apps. Hardware and settings are tabs on
System, next to the timestamped log. A case id from `/new` is the
investigation. Switching the canvas switches which transcript the prompt
appends to.

`Store::call_states` exists only in tests. The tool-call table is written in
every build; the inspector is not part of the library surface.

Hardware detection shells out the same way a host probe has to: `sysctl` and
`system_profiler` on Apple Silicon, `nvidia-smi` when it is on `PATH`, and
`sysinfo` for CPU, RAM, and disks. The Metal budget helper is pure and tested
without a Mac GPU.

An investigation calls `search::research` before the model speaks. The
question is split with regular expressions into person, organization, domain,
handle, email, and topic. Facts (Wikipedia, Wikidata), web (SearXNG, or
DuckDuckGo, plus Brave and Tavily when a key is set), and news (SearXNG news
and GDELT) run when those stages are on. Domain tools (RDAP, crt.sh, DNS,
Wayback, InternetDB) run only when a domain was extracted. Social tools run
for a topic or handle. GitHub runs for a handle or email. One adapter that
fails does not fail the case. `fetch_page` checks the host before the request
and refuses private, loopback, and link-local answers, including IPv4-mapped
IPv6. The DuckDuckGo HTML parser remains for fixtures; the live fallback is
the instant-answer JSON API.
