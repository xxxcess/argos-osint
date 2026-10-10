# Profile analytics dashboard

Profile keeps the internal `ModuleId::System`, **Overview / System** tabs, and portable **Configs** import/export. Overview starts on **Summary**, followed by Intel, Recon, Atlas, Models and Tools. Its simultaneous dashboard panels replace the earlier focused-only accordion and 35-primary-widget presentation. Telemetry, cohorts, retained DTOs/history, retention, privacy and provider scheduler contracts remain authoritative in [profile-dashboard-and-search.md](profile-dashboard-and-search.md).

## Primary inventory

Exactly **20 primary views** are registered: Intel 4, Recon 5, Atlas 3, Models 4 and Tools 4. Summary reuses Recon outcomes, Atlas backlog, Needs attention and Models latency; its KPIs and attention table do not add inventory entries. Removed primary IDs retain useful measurements in the merged details below. Geographic hot zones remain in Atlas, and Profile has no confidence-distribution or collection-origin primary panel.

| ID | Primary surface | Expanded measurements |
| --- | --- | --- |
| `intel.enrichment` | Tag table and availability meters | Distinct ingestion trend; initial/current confidence and coverage |
| `intel.reports` | Absolute outcome stacks by actual mode | Latest-revision outcomes, wall/active duration; Live waiting/blocked separate |
| `intel.publishers` | Ranked distinct article counts | Domain/share and top-three concentration |
| `intel.freshness` | Ordered delay-bin counts | Missing/future timestamps separate; unequal bins are categorical, not density estimates |
| `recon.outcomes` | Absolute terminal-outcome time columns | Workload, recall, accepted/candidate retention |
| `recon.stages` | Paired execution/wait bars | Actual stages; means in seconds and N |
| `recon.diversity` | Category/Eligible/Corroborated/Shortfall table | Independent successes, attempted tools and reasons |
| `recon.directives` | 100% resolution bars by actual mode, N | Answered/partial/unresolved/blocked/unknown assessments |
| `recon.unresolved` | Full-width Live selectable table | Run/Directive/Reason/Evidence/Next action; progress and full prose in detail |
| `atlas.backlog` | Stage/Unit/Queued/Blocked/Oldest table | Discovery dispositions/errors; articles, packets and memories stay distinct |
| `atlas.cycles` | Absolute terminal-outcome time columns | Mean/p95 cycle-duration lines; Live states separate |
| `atlas.temperature` | Signed diverging bars around zero | Comparable prior/current scores, origin/articles; incomparable N/A |
| `models.capacity` | Quota/concurrency meters and table | Sends/limit, pace, queue/oldest age, cooldown/source |
| `models.performance` | Provider/model hierarchy | Comparable role/request cohorts, attempt errors/429, final failures, duration/N; fallback recovery, roles, causes and amplification trend |
| `models.latency` | Successful-execution p50/p95 lines | First-header/content timing, N and coverage |
| `models.queue` | Enqueue-to-send p50/p95 lines | Measured queue samples and coverage |
| `tools.reliability` | Sortable tool table | Logical/wire calls, cache/zero/error rates, mean/p95; usage, triggers and outcomes |
| `tools.search_health` | Engine table and usable-fetch meters | Valid/zero/challenge/parser/transport, last success/version |
| `tools.evidence` | Yield bars with numerator/denominator | Accepted/cited identities and provenance; multi-tool credit is nonadditive |
| `tools.failure_causes` | Ranked terminal-failure counts | Count/share, tool/category/trigger; one cause per invocation |

Every chart has an equivalent expanded table. The expanded-measurement column describes the metric contract; display only authoritative facts available in the snapshot. Unsupported historical identities, provenance, timing or coverage remain unavailable rather than reconstructed. Canonical report and investigation modes come from recorded facts; labels such as Quick/Deep/Full must not replace them.

## Dashboard preset

The shared components are **AnalyticsCard, TimePlot, DetailTable, Meter and ScrollPane**. `profile_layout::LayoutResult::new(area, page, count, offset)` is a pure geometry result consumed by drawing, focus registration and hit testing. Its `PanelRect` values retain virtual panel height and source offset so a scrolled panel is cropped without squeezing its chart or reflowing its table. `profile_components::{kpi, panel, table_row, clipped}` provides reusable Profile panels; general text, editor and transcript primitives remain in `components`. Layout and drawing read immutable snapshots and perform no SQL, network or model work.

At **160×50**, zero-based terminal rows are:

| Rows | Content |
| --- | --- |
| 0 | Existing `[Home] Profile` shell header |
| 1–2 | Overview / System tabs |
| 3–4 | Summary / Intel / Recon / Atlas / Models / Tools |
| 5 | Period, applied filters, Refresh, update time/timezone, stale/coverage |
| 6–11 | Four bordered KPI cards, with labels, values and denominators/N |
| 12–48 | Scrollable analytic body, 37 rows |
| 49 | Contextual keyboard hints |

The body uses two columns at **120 cells or wider**, a **one-cell gutter**, and aligned edges. At width 160 the columns are 79 and 80 cells. Four KPI cards use near-equal widths with one-cell gutters. Below 120 the body becomes one column in reading order; at 80–119 KPIs are two across, and below 80 they use compact two-column values. Keep chart heights at least 11 and table heights at least 9; reduce chrome/KPI height and scroll the body instead of squeezing panels. The practical minimum is 60×18; smaller screens retain navigation and show a size notice. Focus never collapses another panel.

| Page | Four KPIs, in order | Body rows, left / right |
| --- | --- | --- |
| Summary | Report completion; answered directives; Atlas completion; successful model p95 | Recon outcomes / Atlas backlog; Needs attention / model latency. Paired rows 18 + gutter 1 + 18 |
| Intel | Distinct new articles; body availability; report completion; top-three publisher share | Enrichment / reports; publishers / freshness |
| Recon | Terminal runs; answered directives; unresolved Live; corroboration coverage | Outcomes / directives; stages / diversity; unresolved full width. Paired rows 11 + gutter 1 + 11 + gutter 1 + table 13 |
| Atlas | Terminal cycles; completed share; blocked Live by stage/unit; oldest pending Live | Backlog full width; cycles / temperature shifts |
| Models | Sends; final operation failures; queue p95; active/max Live for selected scope | Capacity / performance; latency / queue |
| Tools | Logical calls; usable search fetches; remote errors; evidence-yield rate | Search health / reliability; evidence / failure causes |

Expanded detail occupies approximately **90% of the viewport** and scrolls independently. Save dashboard focus/scroll before opening and restore it on Esc. Render visible panels only. Plot/table rows do not wrap; ellipsize labels by terminal cells and expose complete prose in detail. The unresolved table starts with 8%/30%/23%/14%/25% column proportions, adjusted for readable minima.

System retains Host, Paths, Refresh hardware and Configs. Summary attention rows show owning app/item/reason/age/action for measured blockers, unresolved directives, stalls, search failures and cooldowns. Row activation uses authoritative owning IDs; missing coverage is acknowledged in the empty state.

## Visual and metric rules

Preserve `theme.rs`: BG `#141414`, SURFACE `#1e1e1e`, BORDER `#535353`, ACCENT `#87bfff`, TEXT `#e8e8e8`, GREEN `#98c379`, WARN `#e5c07b`, RED `#f08a8a`, teal `#50c8c8`, and existing DIM/MUTED/SELECT. Use thin plain borders, one-cell horizontal inner padding, left-aligned ACCENT border titles, and normal-size bold KPI values. Navigation selection uses ACCENT fill and dark text; focus uses an ACCENT border and `• focused`; table selection uses SELECT/TEXT.

Recon outcomes use GREEN for completed with evidence, teal for completed with zero evidence, WARN partial, RED failed and muted cancelled/unknown. Stage execution is ACCENT and wait WARN; bars are paired, not stacked. Latency p50 is ACCENT and p95 teal. Series identity and color remain stable through filters and refresh.

- Expose units, common scale, legend, range and selected-point readout. Absolute bars start at zero with a shared maximum; only explicitly 100% views normalize. Stack allocation uses largest remainder. Summary count/time ticks are sparse, legends appear below plots, and backlog units are text.
- Missing duration buckets are gaps. Count views rebucket by summation with common scales. Duration views page original points because the snapshot supplies percentiles rather than the underlying samples needed to rebucket them; `charts::plot_targets` shares this geometry with draw/hit targets. Never average percentiles, interpolate missing timing or crop history silently. Suppress p95 below the existing sample threshold. There is no invented p95 target or success threshold.
- Confidence is a score or ×100 percent. Temperature change is signed **points**. Amplification is sends/terminal operations, displayed as `1.8×`, not `1.8%`.
- Capacity is `sends_60s/effective_limit`; concurrency is `active/max`; pace is requests/minute. Unknown/disabled limits show N/A. Overflow shows the exact value and a warning. Never sum overlapping quota scopes or unlike backlog units.
- KPI rates sum eligible numerators/denominators. State N, coverage and cancellation treatment. Live ignores the period and is labeled. Evidence totals are deduplicated or N/A. Previous-window deltas require comparable coverage; percentage differences use percentage points (`pp`).

## State and interaction

Tab/Shift+Tab follow controls and panels from top-left to bottom-right and reveal the next panel. Arrows select rows or buckets. PgUp/PgDn and wheel scroll focused content, then the page consistently; keyboard and mouse use the same rectangles. Enter expands, and detail-row Enter opens the owner. `v` switches chart/table in detail, Esc restores prior focus/scroll, `f` opens filters, `r` refreshes and `?` opens help. Global shortcuts and picker text retain their existing owners.

Periods are 1h / **24h default** / 7d / 30d / custom. Custom input is `from | to`, with both timestamps in RFC3339 and explicit timezones, for example `2026-10-09T00:00:00-04:00 | 2026-10-10T00:00:00-04:00`. Validate dates and ordering before applying. Existing app/provider/role/mode/tool/category filters apply where supported; show applied chips and unsupported scopes. Expanded tables use those dimension filters, `s` cycles sort columns, `g` folds or expands the selected provider’s model rows, and stable-ID selection persists. Selected-row detail exposes complete prose. Anchored See more appears only for hidden rows, opens detail and disappears at the bottom.

Snapshots refresh automatically every 20 seconds in the background; manual refresh and filter changes request a fresh snapshot immediately. Refresh retains stale data and per-panel selection/scroll while loading. Detail lookup is lazy, bounded and cached by filters/revision. Missing historical facts remain unavailable; add a migration only for necessary authoritative facts, and preserve scheduler ownership.

The metric implementation fills the existing model operation/performance joins, corrects overlapping raw Intel aggregation, keeps Atlas backlog Live independently of the period, and adds snapshot summaries/attention from retained facts. These changes require no schema migration. They do not create historical measurements that an installation never collected.

## Verification and delivery

Fixture checks cover all 20 unique IDs and per-page counts, shared scales (1 versus 100), stack allocation and denominators, unknown/overflow quotas, 1×/2× amplification, signed deltas, missing/low-N lines, stable colors, Unicode/no wrapping, focus/scroll restoration and mouse parity. Also regress System/Configs, global shortcuts, full-record scrolling and stale/empty/loading states.

Render real terminal fixtures through `scripts/render_tui_cells.py` at **160×50, 120×40, 100×32, 80×24, 60×18** and below minimum. Compare actual Summary/Recon output with approved references `exec-b07bd433-8370-4ae7-80de-ff3c11499435.png` and `exec-bb24eaf4-e7ba-4a5d-96da-5416c3469787.png` when available. Reference images contain sample data; metric definitions above override erroneous labels. Concept images are not runtime QA.

Run fmt → clippy → test with `ARGOS_EMBED` unset. OpenCode build/clippy/test commands use `--locked --no-default-features` as specified in [AGENTS.md](../AGENTS.md). Report actual check results, screenshot paths, justified migrations and unverified live behavior; this document specifies acceptance and is not a test result.

Recorded results and retained screenshot examples are in [the implementation session](tui-verification-session-2026-10-10.md). Reproduce the full matrix with [the capture workflow](tui-verification.md); captures belong to a fresh current-plan run, not a fixed historical directory. Artifact generation itself does not establish a reviewed result.
