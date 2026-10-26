# Usage

[Concept route](index.md) · [Wiki](../../index.md)

How to run the terminal UI and CLI. Product tour in the [README](../../../README.md). Investigation mechanics in [architecture.md](../code/architecture.md). Keys and model roles in [providers.md](providers.md).

## Terminal UI

```sh
cargo run -p argos-osint-bin
# Or skip the service menu:
cargo run -p argos-osint-bin -- stack up --tui
```

With no arguments, Argos opens its centered **ARGOS VENTURE** Ratatui service launcher: **TUI**, **API Server**, or **Web**. Select with ↑/↓, Tab or a number and press Enter, or click a choice. The wizard asks for storage mode, ports and the selected service's options; Ctrl+U clears a field, PgUp/PgDn review settings, and Esc cancels. The final choice starts the stack with either the normal TUI or server logs visible. API/Web prerequisites are in the [stack guide](../code/intel-web.md#running-locally). Web opens the ready URL through the default browser; `--no-open` disables this. Explicit CLI commands bypass the wizard, and redirected input retains text prompts.

The TUI opens on **Home**. `↑↓` + Enter open an app. Keys `1`–`9` jump Intel → Profile. Esc or Home returns Home. `?` opens shortcuts.

[![TUI shell](../../diagrams/tui-shell.svg)](../../diagrams/tui-shell.html)

| Keys | Action |
| --- | --- |
| Tab | Next control |
| Ctrl+U / Ctrl+D | Scroll the focused pane |
| Mouse wheel | Scroll the pane under the pointer |
| Ctrl+K | Command palette |
| Ctrl+C | Cancel a running turn, or clear a draft |
| Ctrl+Q | Quit (confirm within one second if work is pending) |

A click commits on mouse-up only if the pointer stays within one cell of the press. Dragging across Home does not launch an app.

With an API/Web stack and **TUI** terminal display, open **Jobs**, select **Rust API server** and activate **View logs** (`l`) for API traffic. Web stacks also list **Next.js web server** (development) or **Static web server** (built export). These service jobs use the shared embedded repository and show access logs/server output through the Rust Logs app. Operational Jobs/Logs are not part of the React Native Intel client.

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

Recall defaults to **off** for each Recon investigation transcript. Enabling its Recall control permits Brain recall and publication/indexing of that transcript’s claims and inferences. Local transcript analysis still runs when Recall is off. Publication checks the live preference, so queued work captured while Recall was on cannot write new Brain memories or index entries after it is turned off. Earlier opted-in memories remain stored.

Turn flow, bindings, and citations: [architecture.md](../code/architecture.md).

[![One Recon turn](../../diagrams/recon-turn.svg)](../../diagrams/recon-turn.html)

### Intel

Atlas headline board: tabs, day filter, search, hero story.

- Enter → Focus Brief: extracted facts, article reader, Related and brief Jobs.
- Left: Confidence, Actors, Claims, Inferences and Links. Activate a Claim or Inference record to map its exact Brain topology.
- Middle: full-width text preview, Visit/Reload, bordered Full Article, then bordered Related.
- Right: Tags, Country and brief preparation Jobs. Compact pages expose Extracted, Reader and Context.
- Related has its own cursor and scroll; focus reveals its panel. Wheel and PageUp/PageDown scroll its rows before the middle stack. See more stays anchored.

Intel Focus Brief shows Confidence above Claims: an Admiralty badge with Source, Information and Brief rating. The deterministic rating averages unique supported claims and inferences, collapses copied support roots and penalizes explicit refutations; it is a support score, not a truth probability. Source-only B6 is 60 provisional, F6 is N/A, empty completed assessment is N/A and all-refuted is 0. Pending/failed replacements retain the previous rating with a label. Actors from stored title/description mentions are visible before claims and during refresh. Scroll the extracted column to reach every Actor; Related is beneath the summary in the reader. The summary pane shows the stored description, with independent scrolling and See more. The full retrieved article body is not drawn there. Tags/Country/Jobs remain visible in the context column.

Visiting a brief starts Full article and Claims independently. Inferences then use actor-mention sentences selected by Classifier for each claim, rather than article chunks. Each claim receives its own analysis request with associated actors and selected evidence; unsupported outlooks are withheld. Claims publish and queue indexing as soon as extraction completes, Inferences publish next, and Links publish last, each updating only that kind. A later failure leaves earlier results available. Jobs distinguishes extraction progress, publication, current-revision indexing and failure reasons. Article Recon reports are retired: no mode classification, report launch, BLUF or full-report selector. Independent Recon investigations remain available. Historical report artifacts are retained, and validated cached brief artifacts republish on the next normal visit without repeating extraction stages.

The React Native Intel app also starts or attaches brief preparation when you open a Focus Brief, including a Related article. Polling and refresh do not repeatedly submit the same visit. If admission fails, an alert appears above the brief; use **Ensure / resume brief** in Context to retry. Opening an unchanged article reattaches existing work; **Reload article / new generation** remains explicit.

In Intel, Ctrl+K → **Open extraction job** shows stage, completed/total windows and assessment batches, provider route/attempt, withheld items and the exact failure. Ctrl+K → **Retry extraction**, or the extraction job's **Retry extraction** button, admits one retry epoch for recoverable unfinished work and keeps completed compatible checkpoints. Visiting again or restarting does not retry exhausted work. A relevant model-route change admits replacement work using the captured old routes for comparison. Qualified items can publish while optional assessments remain withheld; missing mandatory coverage or all-withheld model failure keeps the previous snapshot. **Published** means Brain rows committed. **Indexed** means their current revisions were acknowledged; pending/disabled index work stays separate. Use **Retry index** on the failed index job in Jobs; reindexing cannot create insights that never published.

Jobs provides **Clear completed**, **Clear stopped** and **Clear failed** to remove the selected status cohort globally, across filters and pages. Stopped includes cancelled and superseded jobs; Failed includes failed and partially completed jobs. A tree with running descendants is retained. **Stop job** (or `c`) safely stops the selected supported running job, dequeues a queued job, or cancels its scheduled retry; its history remains until cleared. Age/owner cleanup is automatic at startup and in five-minute sweeps. All operational jobs expire strictly after 72 hours from creation; updates and retries cannot extend that deadline. Deleted required owners retire younger work immediately. Completed report results and committed Brain insights remain accessible, and canonical index removals continue. A log link to a purged job displays “Job history expired.” **Summary flush worker** and **Local index worker** are long-lived services that remain available while Argos runs, even with no pending tasks. Their service age is not the duration of one request; inspect task attempts, failures and heartbeat for work status.

Confidence retains the original large ASCII Admiralty grade and color gradient beside the score, stacking them at narrow widths. Legacy grades remain intact; new evidence scores use information grade 6 (cannot be judged), since the evidence rubric does not assign an Admiralty truth rating.

Details: [concepts.md](glossary.md) · [architecture.md](../code/architecture.md#intel-reports).

[![Intel brief chain](../../diagrams/intel-brief-chain.svg)](../../diagrams/intel-brief-chain.html)

### Brain

Kind cycles **All → Claims → Actors → All**. All is exactly the union of visible claims and normalized Actor-name groups; internal inferences, links, tentative Related records, digests, profiles and notes stay stored for recall. Create saves a user-supplied, unassessed Claim. Pin/Delete apply to claims; actor groups are read-only.

An Actor group collects all member and redirected identities for browsing. Possessive spellings share a normalized group; article totals include direct retained mentions even when there are no Related articles. Counts follow ingestion, corrections and deletion. The background Actor identity maintenance service repairs old spellings; when Entity Resolver is configured it also assesses contextual aliases. Grouping itself does not merge identities. Thread detections are recorded only with Recall enabled; later disabling Recall preserves earlier receipts.

Enter on a memory (or an actor group) opens its Memory Topology directly, at one hop from the selected record; there is no separate detail or claim-path graph view. Back/Esc returns to the Brain list with the same selection. Click selects a row; Enter opens it. A saved last-view session that recorded a memory detail resumes at the Brain list.

#### Brain Memory Topology

Press Enter on a Brain memory (one hop). In an
Intel Focus Brief, activate an individual Claim, Inference or Actor record (two hops). Each entry opens
its own retained neighborhood; no global sample graph is substituted.

- **Layout**: Graph first puts the canvas left (65%) and inspector right (35%) in a body of at least 132×32 cells; 100–131 columns uses 70%/30%. Shorter/narrower bodies expose Graph / Details / Controls pages. Below 60×18 retain Back and a size notice.
- **Inspection**: Select a node to inspect its complete record and spaced relationships. Select a relationship for true direction, rationale and exact receipts. Inferences and derived topics are visible; actor name groups preserve member identities. Tentative Related leads remain possible context.
- **Navigation**: Recenter (or `c`) changes the seed; selection alone keeps it. Depth 1/2/3 and retained/current-cycle scope change the queried neighborhood. `+`/`-` zoom, Shift+Arrows pan and Reset view clears the transform. Crowded hop columns can be panned. Back/Esc restores topology history and then the Brain/Intel entry. Open record exposes full snapshot detail. Scope analysis remains an explicit separate action.

Home app items, command palette commands, Atlas cycles/article lists and Intel
category headline lists have no blank rows between records. Section boundaries
and all other record spacing remain unchanged.

[![Brain memory topology](../../diagrams/brain-topology.svg)](../../diagrams/brain-topology.html)

[![Brain browse projection](../../diagrams/brain-browse.svg)](../../diagrams/brain-browse.html)

### Atlas

Automatic news cycles run every **30 minutes**. A manual start resets an armed timer to 30 minutes. Restoring an older schedule shortens a later deadline while preserving due triggers and Auto off. An active cycle is never overlapped.

Classification-only news cycle:

1. GNews + NewsData, 48 h, score countries
2. NewsAPI + Currents headlines for kept bands
3. Classifier batches overlap local entities and publisher priors; Related matches shared title/description mentions

Pause preserves article receipts. Resume retries unresolved Classifier/native metadata work. History and the Braille map follow the selected cycle. Coverage shows classification, entities, publisher priors and Related leads. Atlas alone creates no Brain insights; article analysis begins in Intel. See [classification and Related](atlas-classification-and-related-articles.md).

Pipeline and quotas: [architecture.md](../code/architecture.md#atlas).

[![Atlas news cycle](../../diagrams/atlas-pipeline.svg)](../../diagrams/atlas-pipeline.html)

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

Overview uses four KPI cards and simultaneous chart/table panels, following the [dashboard contract](../code/profile-analytics-dashboard.md), [design contract](../code/tui-design-spec.md) and [component catalog](../code/tui-components.md). At body width 120 or wider panels use two columns; narrower screens use one column and scroll. Expanded details occupy approximately 90% of the viewport and keep independent state. Period defaults to 24h; 1h/7d/30d/custom and applicable app/provider/role/mode/tool/category filters show their scope and coverage. Live metrics ignore period. System provides scrollable Host and Paths; below the practical 60×18 minimum navigation remains with a size notice. Typing inside pickers belongs to the search field; Tab changes filter dimension, arrows select a recorded value and Enter applies it.

Custom period input is `from | to`: both timestamps use RFC3339 with explicit timezones, for example `2026-10-09T00:00:00-04:00 | 2026-10-10T00:00:00-04:00`. Expanded tables use the same dimension filters; selected-row detail shows complete prose, and unsupported historical facts display unavailable.


**Configs** (`x`) — Export writes the portable schema-v1 document; Import merges one in. The file contains saved API keys, so treat it as a secret and never commit it.

| Section | Keys |
| --- | --- |
| Export | Type the destination (`~` expands), focus Export and activate it. Activate Export again to confirm overwrite of the same destination |
| Import | Paste the document; Enter inserts a newline. Focus Verify to validate, then Save and apply to commit. Any edit requires verification again |

The redacted change summary lists what moves before you commit. Primary IDs, filters and layouts are documented in [profile-analytics-dashboard.md](../code/profile-analytics-dashboard.md); the metric dictionary, portable configuration contract, validation and commit remain in [profile-dashboard-and-search.md](../code/profile-dashboard-and-search.md).

### Other apps

| App | Role |
| --- | --- |
| Brain | Claims and unique Actor names, Find, Create Claim, typed associations, read-only graph ([recall](../../diagrams/brain-recall.html)) |
| Tools (`Osint`) | Manual HTTP tools, keys, documentation, history |
| Models (`Providers`) | Defaults (primary + ordered fallbacks), OpenRouter, Google, Nvidia |
| Jobs | Background work ([lifecycle](../../diagrams/jobs-lifecycle.html)) |
| Logs | Durable events |
| Profile (`System`) | Activity dashboard, host and storage paths, portable configuration |

Module ids vs labels: [concepts.md](glossary.md). TUI contracts: [conventions.md](../code/conventions.md). Surface audit: [ui-interaction-audit.md](../../archive/ui-interaction-audit.md).

### Insight progress

Intel's Focus Brief and Jobs detail show the streamed insight status (`Insights · Receiving output`, `Stalled`, `Retry wait`, `Published`, `Indexed`) plus an `Activity:` line with its age. Replaced work shows `Superseded · replaced by <job> · <reason>` rather than an error; the Logs view lists `insight.activity` and `insight.superseded` events.

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
| `storage/surreal/` | Embedded SurrealDB: records, provenance, graph and vectors |
| `storage/config.json` | Persisted embedded/server mode configuration |
| `config.toml` | Role defaults |
| `auth.json` | Credentials (owner-only on Unix) |

`ARGOS_EMBED=0` skips MiniLM (Jaccard recall).

[![Persistence stack](../../diagrams/persistence.svg)](../../diagrams/persistence.html)

Persistence: [architecture.md](../code/architecture.md#persistence).


Profile opens Overview; its upper buttons select Overview, System or Configs. `x` and the Configs palette command select the Configs page. Export precedes the multiline Import editor: Verify shows a redacted change summary, then Save and apply commits that exact revision. Enter in JSON inserts a newline. Tools has category groups and compact Catalog / Documentation / Test pages; search reveals matches without changing saved expansion. Intel's Visit article site action opens the article URL. Ctrl+K lists recent commands and current-app shortcuts above universal actions. Recon exposes Jump to latest and previous/next-turn commands through the palette.
## Intel article images

The core terminal TUI displays article text without downloading or rendering images. Use **Visit article site** to open the original article in your browser.

The React Native Intel category feed displays available article images to the left of the headline, description and source information. Thumbnails stay on the left at compact widths. Missing, unsafe or failed image sources collapse so article text and **Open Focus Brief** remain usable. The web static host permits HTTPS images.

Local NLP is configured in `config.toml` with `[nlp] mode = "auto"` (default), `"off"`, `"annotate"`, or `"select"`. It is not part of portable Profile configuration. See [nlp-preprocessing.md](nlp-preprocessing.md) for evidence and fallback behavior.

Related article eligibility requires **at least two distinct overlapping entity mentions** in the exact source/candidate headline or description. Repetitions and aliases of one normalized name count once; country/category fields do not count as mentions. Atlas ingest persists Related associations; Intel Related reflects those retained pairs and rechecks current revisions. Brain Actor Articles instead counts direct retained actor mentions, observed links and claim origins, without requiring a second article or a two-entity pair. A browse visit adds no name-only or canonical-actor suggestions. Original claim Sources preserve provenance independently. Investigation entity associations require Recall on; later disabling Recall preserves earlier receipts. The local match receipt is `shared-headline-entities-v4-possessive`; changing this policy invalidates only Related matching and the dependent Links shortlist, preserving compatible Body/Claims/Inferences and fidelity checkpoints.

## Local Intel web workspace

The [web/API guide](../code/intel-web.md#running-locally) covers `argos stack up --web`, API-only `--server`, static export builds, connection settings and safe shutdown. `argos` and `argos stack up --tui` continue to open the terminal directly. The web application is local development only; public deployment and native clients remain later phases in the [living specification](../../argos-intel-living-spec.md).
