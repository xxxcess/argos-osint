# Architecture

`argos-osint-core` contains Brain recall and persistence, provider connections, and hardware profiling. `argos-osint-bin` contains the CLI and terminal UI.

The terminal retains a left app launcher, right canvas, and bottom composer. Brain is a top level app. Providers has Grok, OpenAI, OpenRouter, and Models tabs. System shows hardware and paths. The shared palette and rounded panels are in `tui/theme.rs`. The TUI enables terminal mouse capture; `tui/ui.rs` defines both rendering and hit areas, while `tui/app.rs` routes mouse and keyboard focus to fields, tabs, rows, and buttons.

Grok and ChatGPT sign-in run as background tasks and send progress and completion events to the TUI. OpenRouter verification checks a draft account without storing it; Save stores that account without changing the other providers. Writer routing is saved separately in `config.toml`.

Brain's `MemorySource` requires an app and conversation ID and can carry a message ID and reference. `Store::add_memory` validates provenance, and `Store::recall` returns ranked memories with the same metadata. This API is the integration point for future chat apps. The `argos remember` and `argos recall` commands expose the same operations as JSON friendly CLI paths.

The SQLite migration keeps ordinary memories, marks their unknown legacy origin, and removes old case, report, and research tables. New records use the `source_json` column.
