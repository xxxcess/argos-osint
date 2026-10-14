# Database responsiveness verification — 2026-10-14

This verifies the worker ownership and cached presentation described in
[TUI database and index responsiveness](tui-database-responsiveness.md). Changes
use the existing Brain list/detail, Atlas history/news, Intel bulletin/brief,
Recon transcript, Jobs and Logs components and presets. Profile geometry,
shell/theme geometry and navigation bindings are unchanged.

## Reproduce

Use the dependencies and image review procedure in [TUI verification](tui-verification.md).
Fresh captures use the agent Cargo cache and temporary state, with `ARGOS_EMBED`
unset:

```sh
python scripts/tui_review.py capture --output .planning/<task>/tui/loading \
  --test dump_db_responsiveness_screens \
  --include 'brain-*' --include 'intel-bulletin-loading-*' \
  --include 'atlas-news-loading-*' \
  --size 160x50 --size 120x40 --size 100x32 --size 80x24 --size 60x18 --size 40x20 \
  --reason 'Background reads: initial loading, retained refresh/error rows, Intel and Atlas'
python scripts/tui_review.py capture --output .planning/<task>/tui/brain \
  --test dump_brain_kind_screens --ignored \
  --include 'brain-actor-graph-*' --include 'brain-claim-split-*' \
  --size 160x50 --size 120x40 --size 100x32 --size 80x24 --size 60x18 --size 40x20 \
  --reason 'Shared Actor association snapshot and Claim detail delivery'
python scripts/tui_review.py capture --output .planning/<task>/tui/related \
  --test dump_atlas_related_screens --include 'intel-related-*' \
  --size 160x50 --size 80x24 \
  --reason 'Retained Related pages and loading/unavailable state after worker delivery'
python scripts/tui_review.py capture --output .planning/<task>/tui/consumers \
  --test dump_overhaul_screens --include 'app-4-*' --include 'app-5-*' \
  --include 'recon-transcript-*' --size 160x50 --size 80x24 \
  --reason 'Jobs, Logs and Recon consumers of background snapshots; unchanged geometry'
python scripts/tui_review.py capture --output .planning/<task>/tui/dashboards \
  --test streamed_insight_progress_screens --include 'stream-jobs-3-receiving-*' \
  --include 'stream-logs-*' --size 160x50 --size 120x40 \
  --reason 'Loaded Jobs detail and durable Logs after snapshot delivery'
```

Captures record the implementation, renderer, font and artifact hashes in their
manifest. Generated pixels reconstruct actual ratatui TestBackend cells; they
are not live terminal photographs. Visual, interaction and association results
are assessed separately below.

## Visual results

All **60 selected PNGs** were inspected with an image-capable viewer, using
contact sheets and individual retained images. No new overlap, palette drift or
lost selection was observed in the affected states. Geometry is unchanged;
shared scheduling is covered by representative consumers and interaction tests,
so unrelated Profile/Configs pages were not added to the visual sweep.

| Scope and exact fixture stems | Sizes | Observed result |
| --- | --- | --- |
| `brain-loading-*`, `brain-refreshing-*`, `brain-read-error-*`, `intel-bulletin-loading-*`, `atlas-news-loading-*` | 160×50, 120×40, 100×32, 80×24, 60×18, 40×20 | 30 images. Loading labels are visible; refresh/error retain the selected Claim. Controls and wrapping fit supported sizes. |
| `brain-actor-graph-*`, `brain-claim-split-*` | Same six sizes | 12 images. Actor Graph/Related share the 45-article total, with 20 rows and See more. Claim metadata, read-only graph, Actors/Sources tabs and saved Summary remain present. Compact pages retain their controls. |
| `intel-related-articles-*`, `intel-related-empty-*`, `intel-related-loading-*`, `intel-related-unavailable-*` | 160×50, 80×24 | 8 images. Related selection, scope, source basis, See more and distinct empty/loading/error messages remain visible. |
| `app-4-*`, `app-5-*`, `recon-transcript-*` | 160×50, 80×24 | 6 images. Jobs/Logs filters, cleanup controls, follow state and Recon transcript/input retain their layout. |
| `stream-jobs-3-receiving-*`, `stream-logs-*` | 160×50, 120×40 | 4 images. Selected Jobs detail and four durable Logs rows remain legible. Synthetic Jobs rows do not certify header rollups or unavailable timing. |

At the supported 60×18 minimum, the list/graph viewport is small and additional
rows or detail pages require navigation. The exploratory 40×20 cases retain
existing label/footer clipping below that minimum; they are not a certification
of a redesigned small-screen layout. Courier New, identified in the capture
manifest, shows fallback boxes for the Recon CJK fixture. Raw cells retain
Unicode; live font fallback was not verified.

Four distinct states are retained with hashes in
[the evidence manifest](screenshots/2026-10-14-db-responsiveness/manifest.json):

- [Brain refresh retaining selection](screenshots/2026-10-14-db-responsiveness/brain-refreshing-160x50.png)
- [Brain read failure retaining rows](screenshots/2026-10-14-db-responsiveness/brain-read-error-80x24.png)
- [Actor graph and shared article total](screenshots/2026-10-14-db-responsiveness/brain-actor-graph-160x50.png)
- [Intel Related loading](screenshots/2026-10-14-db-responsiveness/intel-related-loading-80x24.png)

## Interaction and data results

The regression suite covers cached Brain selection/Find, stale delivery and a
failed read retaining the last good list; pending detail navigation, Back and
Escape; a requested Jobs destination surviving snapshot delivery; failed Logs
clearing retaining its view while navigation proceeds; and bounded event bursts
leaving time for keyboard input. Existing Atlas cycle deletion and Related scope
tests remain part of the workspace suite.

Recalled-memory popup summaries are captured on the Recon reader worker. The
regression replaces its database connection after delivery and still renders
the retained summary and evidence count. The Logs test focuses a button before
using numeric app navigation; digits in the search field remain text input.

The fixed browse cohort contains 150 article occurrences and 250 Actor-name
groups. Shared Council/Port names receive three articles; each private Witness
receives zero. Batched eligibility is compared with the existing occurrence
validator. This verifies ingest authority and the two-entity minimum without
changing identity resolution or original provenance.

The projection took **294 ms** in the isolated, in-memory cohort. This is an
observed test timing, not a latency guarantee or a before/after comparison with
the changed live database.

| Check | Actual result |
| --- | --- |
| `cargo fmt --all --check` | Passed on final implementation. |
| Isolated strict workspace Clippy, `--locked --no-default-features` | Passed. |
| Same Clippy with `--features nlp` | Passed. |
| Offline workspace tests, `--locked --no-default-features -- --test-threads=4` | 230 TUI, 988 core and 3 integration tests passed; 18 ignored. |
| Full NLP workspace tests during the audit | 228 TUI, 993 core and 3 integration tests passed; 18 ignored, before the final popup/Logs/lease-helper follow-ups. |
| Final implementation with NLP: `scheduler::tests` and `nlp::` | 7 scheduler and 13 NLP tests passed; the NLP performance test remained ignored. |
| Fixed browse cohort, `batched_browse_counts_preserve_ingest_authority_for_large_cohorts` | Passed; 250 Actor groups and authoritative article totals. |

The short three-second lease contention test failed once in the default-parallel
full run, passed in isolation, then passed in the final complete four-thread
run. Its assertion and production lease behavior were not weakened. These
checks use `scripts/agent_cargo.py` and `target/agents`; localhost access was
enabled only where mock provider tests required it.

## Limits

There is no approved visual baseline for these loading states. Supplied live
Atlas/Intel screenshots describe the reported symptoms and provider error;
comparison uses the maintained component and viewport contracts. Live terminal,
live provider, default LanceDB and MiniLM checks are not part of this offline
verification. The original failed entity-inventory cohort disappeared before
replay, so its precise parser failure remains unverified; future receipts retain
bounded failure flags. External provider errors and token truncation are still
possible after correcting the Google native schema MIME enum.

The original live browse measurement (434 rows, 84,334 ms) cannot be compared
with a second live timing because that database changed. Graphify updated the
tracked code graph; SQL extraction was unavailable because `tree_sitter_sql`
was not installed, and semantic document extraction was not configured.
