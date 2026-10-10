# Usage

How to run the terminal UI and CLI. Product tour in the [README](../README.md). Investigation mechanics in [architecture.md](architecture.md). Keys and model roles in [providers.md](providers.md).

## Terminal UI

```sh
cargo run -p argos-osint-bin
```

Opens on **Home**. `↑↓` + Enter open an app. Keys `1`–`9` jump Intel → Profile. Esc or Home returns Home. `?` opens shortcuts.

[![TUI shell](diagrams/tui-shell.svg)](diagrams/tui-shell.html)

| Keys | Action |
| --- | --- |
| Tab | Next control |
| Ctrl+U / Ctrl+D | Scroll the focused pane |
| Mouse wheel | Scroll the pane under the pointer |
| Ctrl+K | Command palette |
| Ctrl+C | Cancel a running turn, or clear a draft |
| Ctrl+Q | Quit (confirm within one second if work is pending) |

A click commits on mouse-up only if the pointer stays within one cell of the press. Dragging across Home does not launch an app.

### Recon

Full-screen list of recent investigations. Enter opens the transcript. Esc from the transcript returns to the list.

| Keys | Action |
| --- | --- |
| Tab | Transcript ↔ prompt |
| Enter / Shift+Enter | Send / newline |
| `↑↓` / `←→` | Select message · fold decision/tool |
| `f` | Full text |
| Ctrl+N | New investigation |
| Alt+Left / Alt+Right | Cycle threads (cursor not in a field) |

Question on a raised prompt band. Answer is Markdown. Decisions and lookups stay collapsed. `◉ brain` opens memories used in that synthesis. Untitled until Recon names it from the first question. Composer commands: `:rename`, `:delete`, `:delete-with-insights`.

Turn flow, bindings, and citations: [architecture.md](architecture.md).

[![One Recon turn](diagrams/recon-turn.svg)](diagrams/recon-turn.html)

### Intel

Atlas headline board: tabs, day filter, search, hero story.

- Enter → Briefing Focus (claims, tags, links, mini-map, Summary)
- View full report → 70%-width card
- Mode button → Verify / Explain / Assess Outlook / Full Assessment (Jobs-backed)
- Busy work on this article hides Summary and the launch control

Details: [concepts.md](concepts.md) · [architecture.md](architecture.md#intel-reports).

[![Intel report job](diagrams/intel-report.svg)](diagrams/intel-report.html)

### Atlas

Two-phase news cycle:

1. GNews + NewsData, 48 h, score countries
2. NewsAPI + Currents headlines for kept bands

Pause stores a cursor. Resume continues and retries only incomplete packets. History + Braille world map follow the selected cycle (default world view about 1.20× the previous scale). Insights tables wrap to inner width. Incomplete indexing stays **waiting** while background retries continue; optional context failures complete with warnings.

Pipeline and quotas: [architecture.md](architecture.md#atlas).

[![Atlas news cycle](diagrams/atlas-pipeline.svg)](diagrams/atlas-pipeline.html)

### Profile

Overview starts at Summary, followed by Intel, Recon, Atlas, Models and Tools. Its simultaneous panels use exactly 20 primary views (4 / 5 / 3 / 4 / 4); Summary reuses existing views. Tab/Shift+Tab traverse controls and panels in reading order; Enter activates, and `t` switches Overview/System.

| Keys | Action |
| --- | --- |
| `0`, `1`–`5`, `[` / `]` | Summary, app views, previous/next page |
| Arrows / `j` / `k` | Select rows/buckets or panels |
| Enter / `m` | Expand detail; detail-row Enter opens its owning app/item when available |
| Left/Right in report | Select original buckets or page duration points |
| PageUp/PageDown / wheel | Scroll focused content, then the page |
| `v` in report | Switch chart/table |
| `s` in report | Cycle sort column |
| `p`, `f`, `c` | Choose period, edit six dimensions, clear dimensions |
| `r` | Refresh analytics in Overview, hardware in System |
| `x` | Configs |
| Esc | Restore dashboard focus/scroll or return Home |

Overview uses four KPI cards and simultaneous chart/table panels, following the [dashboard contract](profile-analytics-dashboard.md), [design contract](tui-design-spec.md) and [component catalog](tui-components.md). At body width 120 or wider panels use two columns; narrower screens use one column and scroll. Expanded details occupy approximately 90% of the viewport and keep independent state. Period defaults to 24h; 1h/7d/30d/custom and applicable app/provider/role/mode/tool/category filters show their scope and coverage. Live metrics ignore period. System provides scrollable Host and Paths; below the practical 60×18 minimum navigation remains with a size notice. Typing inside pickers belongs to the search field; Tab changes filter dimension, arrows select a recorded value and Enter applies it.

Custom period input is `from | to`: both timestamps use RFC3339 with explicit timezones, for example `2026-10-09T00:00:00-04:00 | 2026-10-10T00:00:00-04:00`. Expanded tables use the same dimension filters; selected-row detail shows complete prose, and unsupported historical facts display unavailable.


**Configs** (`x`) — Export writes the portable schema-v1 document; Import merges one in. The file contains saved API keys, so treat it as a secret and never commit it.

| Section | Keys |
| --- | --- |
| Export | Type the destination (`~` expands), focus Export and activate it. Activate Export again to confirm overwrite of the same destination |
| Import | Paste the document; Enter inserts a newline. Focus Verify to validate, then Save and apply to commit. Any edit requires verification again |

The redacted change summary lists what moves before you commit. Primary IDs, filters and layouts are documented in [profile-analytics-dashboard.md](profile-analytics-dashboard.md); the metric dictionary, portable configuration contract, validation and commit remain in [profile-dashboard-and-search.md](profile-dashboard-and-search.md).

### Other apps

| App | Role |
| --- | --- |
| Brain | Saved memories, Find, Create / Pin / Delete, path graph ([recall](diagrams/brain-recall.html)) |
| Tools (`Osint`) | Manual HTTP tools, keys, documentation, history |
| Models (`Providers`) | Defaults (primary + ordered fallbacks), OpenRouter, Google, Nvidia |
| Jobs | Background work ([lifecycle](diagrams/jobs-lifecycle.html)) |
| Logs | Durable events |
| Profile (`System`) | Activity dashboard, host and storage paths, portable configuration |

Module ids vs labels: [concepts.md](concepts.md). TUI contracts: [conventions.md](conventions.md). Surface audit: [ui-interaction-audit.md](ui-interaction-audit.md).

## CLI

From source: prefix `cargo run -p argos-osint-bin --`. Same store, registry, and executor as the TUI. JSON where practical.

```sh
# Recon
argos recon new --title 'Example investigation'
argos recon list --search example
argos recon show <thread-id>
argos recon ask <thread-id> 'What is known about example.org?'
argos recon ask-new 'What is known about example.org?'
argos recon resume <run-id>
argos recon retry <run-id>
argos recon delete <thread-id> --with-insights

# Tools
argos osint list
argos osint describe shodan_internetdb
argos osint run shodan_internetdb --input '{"ip":"8.8.8.8"}'
argos osint history
argos osint attach <call-id> <thread-id>
argos osint user-agent 'Argos contact@example.com'

# Models
argos defaults show
argos defaults set recon --provider openrouter --model <model-id>
argos defaults set tool-picker --provider openrouter --model typesafe/jev-1.13
argos defaults set synthesis --provider openrouter --model <model-id>
argos models --role recon

# Brain
argos insights --entity example.org
argos remember --app research --conversation thread-123 'A manually saved fact'
argos recall 'What do I know?'
argos memories
argos memories reindex
```

- `recon show` — plan (directives, grounding, bindings, picker records)
- Default delete keeps Brain memories and marks their source deleted; `--with-insights` removes memories owned only by that investigation
- `recon limits` — per-turn budgets (`--max-calls`, provider credits). `--turn-seconds` / `--max-turn-seconds` are hidden and deprecated; they no longer terminate a turn.
- `argos ask` streams to stderr; stdout is final JSON
- Nominatim and SEC need an identifying User-Agent first

## State

State: `~/.argos` (`ARGOS_HOME` overrides).

| Path | Contents |
| --- | --- |
| `argos.db` | SQLite, `user_version` 26, additive migrations |
| `memory_lancedb/` | Brain vectors |
| `config.toml` | Role defaults |
| `auth.json` | Credentials (owner-only on Unix) |

`ARGOS_EMBED=0` skips MiniLM (Jaccard recall).

[![Persistence stack](diagrams/persistence.svg)](diagrams/persistence.html)

Persistence: [architecture.md](architecture.md#persistence).


Profile opens Overview; its upper buttons select Overview, System or Configs. `x` and the Configs palette command select the Configs page. Export precedes the multiline Import editor: Verify shows a redacted change summary, then Save and apply commits that exact revision. Enter in JSON inserts a newline. Tools has category groups and compact Catalog / Documentation / Test pages; search reveals matches without changing saved expansion. Intel's Visit article site action opens the article URL. Ctrl+K lists recent commands and current-app shortcuts above universal actions. Recon exposes Jump to latest and previous/next-turn commands through the palette.
## Intel article image quality

Argos queries terminal graphics capabilities at startup. Direct-session fallback
detection selects Kitty graphics for Kitty/Ghostty and inline iTerm images for
iTerm2/VS Code. In VS Code enable `terminal.integrated.enableImages` (see
[terminal image support](https://code.visualstudio.com/docs/terminal/advanced#_image-support)).
Multiplexed sessions rely on capability queries rather than outer-terminal names.
When pixel-size queries are unavailable, native rendering uses the adapter's
default cell aspect ratio. Apple Terminal and unknown terminals retain the
portable half-block fallback, whose resolution is limited to terminal cells.
For photographic detail use a terminal that supports native image graphics.

Images retain their source resolution during decoding and use a single high-quality
contain resize for display. Bulletin imagery and short article previews are
vertically centered; briefing imagery is horizontally centered in the reader column.
