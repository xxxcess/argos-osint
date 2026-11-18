# Matte Canvas interaction and state contract

[Theme](theme.md) · [Motion and startup](motion.md) · [Implementation handoff](../../../.planning/2026-11-16-matte-canvas/handoff.md) · [Native verification](../../wiki/code/native-verification.md)

**Status: approved design target; runtime implementation and verification pending.** This document specifies the approved native and shared web interaction behavior. The documentation change does not implement these behaviors. The audited source baseline is `nightly` at `00b700afd77ddac6cba2e2bd7ae91ab3802b7197` (tree `5051474185a8eb14b38e607480bb24c65352fff0`). Source inspection identifies mechanisms; it does not establish a reproduced temporal defect or a product-test pass.

## Visual and interaction hierarchy

Reproduce the approved [Workstation](mockups/01-workstation-status.png), [Gateway](mockups/02-setup-gateway.png), [Jobs and Logs](mockups/03-jobs-logs.png), and [Compact Workstation](mockups/04-compact-workstation.png) compositions using the shared recipes in [theme.md](theme.md). Keep status readable at a glance, then expose useful details through existing section, row and inspector affordances.

- The canvas blends softly into smooth matte cards and rows. Do not add decorative panel strokes, bright enclosing boxes or an extra toolbar to communicate activity.
- Coarse matte moon-rock material is reserved for **Submit, Confirm, Create and Delete** action buttons. Utilities, navigation, toggles, filters, cards and data rows remain smooth matte. An ordinary Retry, Refresh, Back or Load more control must not inherit the coarse action material simply because it is a button.
- The emblem is the persistent symbol-only brand. Only the logo has permanent sheen. Loading may use the restrained cool sheen below; ordinary controls, live badges, values and text do not continually glint or pulse.
- Use cool text and the semantic palette in the theme; do not reintroduce ivory typography. Health is not conveyed by color alone. Preserve visible focus, accessible names, error explanations and input affordances even when removing decorative borders.
- Keep the same surface quality through splash → gateway → service first load → settled/live service. Loading, unavailable and stale content occupy the same component family and exposure as normal content.

## Current mechanisms and selected changes

Native paths below are under `apps/fleetworks-desktop/src/`.

| Observed source | Required target |
| --- | --- |
| `app.rs::view` replaces its connection pill with **Syncing** when the aggregate `any_busy` is true; `context_rail` uses **Working/Ready** from `sidebar_row_status`. | Keep connection health separate from request activity and domain state. A periodic detail read must not rename **Online** or change the section title. |
| `State::refresh_readiness` retains Ready during the request, but `complete_readiness` replaces the last-good value with Failed after an error. `readiness_cards::checks` then substitutes an error strip or skeleton. | Retain usable same-scope checks on a transient refresh failure, mark them stale and preserve freshness-dependent action restrictions. |
| `logs.rs::LogState::ticket` sets Loading for Follow; `backend_views::jobs` chooses a skeleton from `rows.is_empty() && busy()`. | Remember accepted empty snapshots. A successful empty log view remains settled while the next long poll runs. |
| The Jobs view alternates keyed `logs-loading`/`logs-synced` strips; `async_status_strip` conditionally adds a progress row. | Use one stable section status slot. Do not replace keys, headings or height at each request boundary. |
| `Brain::edited` assigns Pending, then `State::search_now` assigns Loading; `views::brain` changes debounce narration into a skeleton. | One unresolved results-shaped presentation through debounce and first fetch. New query identity still fences old results. |
| Native `backend_views` renders Previous/Next/Page for Jobs and cycle history. `JobsQueryState` and `load_cycles_page` keep one visible page. | Continuous scrolling is the default long-list presentation; bounded cursor-backed loading appends or reconciles without replacing the reader's current window. |
| Web `packages/ui/src/Skeleton.tsx::Skeleton` loops opacity; `LoadingState` text owns no timer. | Replace the full-opacity pulse and correct consumer mount/state transitions. A CSS-only change cannot remove the JavaScript loop. |

Preserve existing useful retention in host metrics, cycle reads, Jobs polls, matching selected-job detail and nonempty log follows. Surface the ordinary Jobs poll error retained in `scoped_jobs.message` while `JobsLoad::Ready`: the current Ready projection does not carry that stale explanation into the list. Preserve web cached refresh and generation-specific pending behavior.

## One section presentation model

Derive presentation from the existing authorities instead of introducing a second network owner:

1. **Identity:** connection/backend namespace, registration/workstation origin where required, resource kind, applied filters, sort, selected entity and active generation.
2. **Accepted snapshot:** data or an accepted empty result, version and last successful observation. Row count alone cannot prove resolution.
3. **Request activity:** initial read, same-scope refresh, continuation read or mutation submission.
4. **Domain state:** queued, running, partial, completed, cancelled, failed, blocked or uncertain, from the current authoritative job/generation.
5. **Transport and freshness:** actual connection health, data freshness, recoverable read error and invalidated authority.

Commit content, version and status coherently. A transport request starting is not a domain-state transition. Independent regions may resolve independently, but never show a new success state with an older mismatched payload.

| Presentation | Content | Indicator and resolution |
| --- | --- | --- |
| Initial unresolved | No accepted snapshot for current identity. Preserve headings, known labels, controls and structural bounds. Skeleton only unresolved regions. | One section busy state. Debounce and first read use the same skeleton geometry. |
| Same-scope refresh | Keep accepted rows, values, charts, empty results and current interactions mounted. | Optional activity in a reserved local slot. Retain the last successful timestamp until a new response commits. |
| Continuation/tail read | Keep loaded content and its scroll anchor. Place placeholders only at the unloaded boundary. | One bounded boundary indicator; no whole-list skeleton and no global Syncing label. |
| Authoritative generation | Preserve current-generation lifecycle. Web `StageContent` retains its stage-local queued/running/retry skeleton precedence over older output where required. Preserve independent accepted content, including existing actor chips during cleanup; expose partial output only where the domain contract permits it. | **Queued**, **Running** or meaningful supplied progress. Failed/cancelled/superseded/blocked states resolve to diagnostics. An older generation cannot complete the current one. |
| Mutation pending | Keep inputs and existing content; mark the affected action/field busy without changing its size. | Admission/Queued is distinct from completion. Preserve reconciliation for uncertain outcomes and explicit rollback for any optimistic edit. |
| Settled data or empty | Show the accepted result immediately. Empty requires successful authoritative resolution. | A concise empty line such as **No jobs** or **No log entries**; repeated empty refreshes remain empty. |
| Stale or refresh failure | Retain usable same-scope data with its last-good time. Keep stale-dependent actions appropriately gated. | **Stale** or **Update failed**, with local recovery and details. A first-read failure replaces unresolved placeholders with a bounded error state. |
| Authority invalidated | Remove content that is no longer authorized for the current namespace. | Appropriate reconnect/scope/access action. Never fall back from workstation Jobs/Logs to all-backend activity. |

Unknown is not zero: render unavailable/unresolved values as **—** with an accessible explanation, and authoritative zero as **0** with its unit. Failure is not empty; a cache/window limit is not the server's end of results. Preserve provenance and the distinction **Related leads · not corroboration** when shortening copy.

### Fences and precedence

Reuse the native epoch/revision checks in readiness, host metrics and backend reads; cycle epoch/origin/revision/pending-request checks; Jobs binding/generation/sequence checks; Logs binding/generation/sequence/action checks; and Brain query/detail revisions. Keep web connection namespaces, query keys, abort signals, expected-context revisions and active-job validation.

Only an accepted response may settle presentation. Superseded callbacks must not clear a newer indicator, erase its error, reset its cursor, or mark the wrong output fresh. `complete_scoped_logs` calls controller completion and then updates follow policy; do not mistake an unconditional post-callback action for proof the response was accepted. Ordinary scope changes abort/invalidate obsolete reads; they must not alter already admitted backend work.

Authoritative terminal completion survives an unrelated live disconnect. Active interrupted work remains visibly interrupted. Queue/running state survives polling. Render successful data and actionable errors in the next committed update: no minimum loader duration, forced shimmer completion or cosmetic completion delay. Coalesce changes already available before one paint without introducing a delay timer.

## Skeleton and copy rules

Use a steady, constant-opacity skeleton base with one broad, dim, translated **cool** sheen per visible section, with the theme's **1.6-second total cycle** (1.2-second sweep plus 0.4-second pause; 1.2–1.8 seconds is the permitted tuning range). Clip it to placeholder shapes. Share phase within the section, preserve its base fill, and never pulse real content or sweep across readable text. Pause decoration when hidden. Reduced motion uses the same static placeholder geometry. Native matte rendering may still use an opaque loading sheen when motion is permitted; it must not force transparency or glass effects.

Skeletons use the final layout primitives: same card padding, column count, row height, value/unit alignment and media ratio. Preserve known chart labels and chart bounds without inventing data. The native `views::workstation` telemetry stacks below 540 logical pixels while `skeleton_metric_tiles` currently always uses a row; both must share the measured decision. Native Brain uses the owner's requested full-width results list with inline, lazy-loaded record disclosures at all widths; keep search/results mounted while expanding or collapsing. Preserve web Brief's 900-pixel reading/context order unless a separately reviewed requirement changes it.

Use stable headings such as **Jobs**, **Logs**, **Connection**, **Output**, **Confidence**, **Actors**, **Claims**, **Inferences** and **Related leads**. A heading must not alternate between Latest, Loading and Updating. Use one local state, icon or count when informative. Remove implementation narration such as **Debouncing catalog lookup…**. Put long diagnostic reasons, timestamps and protocol detail in existing disclosure/inspector surfaces while keeping the error and recovery action discoverable. Reserve space for localized status labels; align numerals so neighboring controls do not move.

Skeleton shapes are decorative and excluded from accessibility navigation. Expose one meaningful section busy state and announce accepted state changes once. Do not announce every poll, repeated unchanged payload or sheen cycle.

## Continuous scrolling contract

### Default and API boundary

Long lists load continuously as the reader approaches their boundary. Remove **Previous**, **Next** and page-number UI from native Jobs/cycles; do not introduce those controls elsewhere. This changes client presentation, not the opaque API cursor protocol. Reuse server ordering and exact returned cursors; never decode them into offsets, invent page arithmetic on the wire or widen authorization to complete a view.

Current native contracts include `jobs_query::PAGE_SIZE = 50`, `HISTORY_LIMIT = 64`, a 40-row cycle read in `State::load_cycles_page`, and `logs::DISPLAY_LIMIT = 500` with 100-row pages. Preserve response validation and existing hard guards unless a separate bounded-contract change is explicitly implemented and verified. Continuous scrolling does not authorize unbounded retention or a claim to show every record.

### Admission, accumulation and recovery

1. Maintain an ordered retained window keyed by the canonical resource/query identity, plus accepted cursor segments and stable row IDs. Native `JobsQueryState::handle`, `begin_ticket`, `complete`, `jobs_effects::project_jobs_page`, and cycle `load_cycles_page`/`complete_cycles_page` need accumulated-window semantics rather than replacement of a single page. Keep selected detail independent of the visible list slice.
2. A near-boundary signal requests at most one next segment when a valid continuation exists and that query has no conflicting read. Use the existing request controller and a deduplicated request key containing identity, generation, cursor and direction/action. Repeated scroll events, rerenders and visibility changes must not submit duplicate reads. A background refresh waits for an active continuation rather than cancelling it or clearing the window.
3. Validate context, sequence and bounded response shape before merging. Deduplicate by canonical entity ID, reconcile changed rows from authoritative newer observations and preserve server order. Native Jobs already keys rows by job ID; web Bulletin uses `keyFor(article)`. Do not use array index, current page number or status text as row identity.
4. Keep accepted content while a continuation is pending. Append or prepend only the accepted segment; boundary skeletons occupy only incoming positions. Refresh reconciles the retained window instead of replacing it with the latest first page. If the server invalidates the cursor/scope, show an explicit restart action and invalidate only according to that authority result.
5. Stop automatic continuation on absent continuation/end evidence, a repeated or previously visited cursor, a no-new-row response, invalid response, failure or cache/display bound. Do not busy-loop through empty/deduplicated pages. Distinguish true **End of results** from **View limit reached**, **Update failed** or an intermediate no-progress pause. An advanced but empty intermediate cursor may remain eligible for one explicit continuation; it is not proof of end.
6. A failed boundary keeps loaded content and its anchor. Pause auto-retry and show a compact inline **Retry** bound to that exact current cursor/query; do not add a toolbar. If automatic visibility detection is unavailable or keyboard users need control, expose an inline **Load more**/continuation fallback with a stable label, focus and busy state. It calls the same single-flight path. Never focus it or scroll it into view merely because a request finished.

Web already has part of this mechanism: `BulletinView` uses a near-end `onScroll` check, `hasNextPage`/`isFetching`/`isFetchNextPageError`, and `fetchNextPage({ cancelRefetch: false })`, plus an inline continuation/retry fallback. `packages/api-client/src/index.tsx::useArticles` owns `useInfiniteQuery`, query identity, cancellation and `getNextPageParam`. Extend these mechanisms; do not create a second feed, interval or subscription just to make scrolling look continuous.

### Memory, virtualization and reader position

Bound retained row payloads, decoded media and cursor metadata. Record numerical resident-page/byte budgets and overscan before source freeze, then measure them. An initial implementation budget of at most eight resident Jobs/cycle segments is a tuning proposal, not a measured capability; keep the current Jobs cursor-history and log display guards. Do not silently raise those limits. If a cap prevents further loading, show that bounded state honestly with the existing narrowing/export route where available.

Virtualize long content using viewport measurements and a small overscan region, retaining stable logical item IDs, measured heights and spacer extents. Evict off-screen payload segments without changing the reader's anchor; rehydrate only through retained valid opaque cursors and the same fences. Pin the focused row, edited input, open details and selected item while needed. If the current runtime lacks the required virtualization affordance, implement a bounded native window adapter over existing scroll/layout facilities, document its cap, and verify anchor behavior; do not import a webview or invent an unsupported Rune API.

Before an insertion, prepend, refresh reconciliation or eviction, capture a visible row ID and its pixel offset. Restore that anchor after layout. Do not reset scroll to the start, automatically select the first row, collapse expanded detail or refocus the reader on every update. Responsive grids change column count only when their measured container changes; live values do not reorder cards or trigger masonry reflow. Loading, resolved, empty and stale states use the same measured slots and semantic/keyboard order.

Logs preserve their current sort/direction and distinguish the continuation boundary from the live edge. Follow newly arriving records only while the reader is already following that live edge. When the reader scrolls away, preserve row/offset and accumulate a quiet unread count in the existing log heading/affordance; returning to the live edge resumes follow. Do not add a permanent log toolbar.

**Implemented in native Logs.** The unread marker row is always reserved, empty at zero so the table does not shift, and shows while the count is above zero a status pill with its text (**N new log lines**) and a labelled **Jump to newest** button above the table. It needs neither color nor motion to read and works from the keyboard. Only followed rows new to the loaded set count, only while the reader is away; a refresh, a repeated row, reaching the newest rows, the jump button, and a scope or backend switch all clear or skip the count. The jump is a plain scroll request, which the runtime performs at once under reduced motion. The away signal is the newest row's own text cell reporting its drawn position against the table top, with no extra column or hidden element; a jump that unmounts that row without first moving it (dragging the scroll bar far) keeps the previous answer until it mounts again. Older-log loading calls the existing `LogState::older` path. A transient follow interruption retains validated rows, and the 500-summary bound remains explicit; it is not replaced with silent dropping, endless retention or a false empty/end state.

## Future acceptance sequences

Use the production native `State`/view/handlers with controlled effects and existing web components; do not validate a separate mock implementation. Candidate native checks are `log_tests.rs`, `jobs_query_tests.rs`, `tests.rs`, `maintenance/tests.rs` and inline `workstation_dashboard.rs`/`views.rs` tests. Candidate web checks include `Skeleton`, `LoadingState`, `Bulletin`, `BriefView`, `BriefProgressStepper`, `IntelShell`, `useIntelQuery` and `live-query` suites.

| Sequence | Observable acceptance |
| --- | --- |
| Initial unresolved → data or empty → repeated same-scope refresh | Stable heading/bounds; accepted empty never becomes first-load skeleton again; unchanged payload causes no remount or word churn. |
| Retained readiness/metrics → transient error → retry → success | Last-good values stay readable, stale restrictions remain truthful, error stops inappropriate shimmer, success resolves immediately. |
| Query A → query B → late A; generation A → B → late completion A | Only current accepted identity updates data, activity, freshness, cursor or domain result. |
| Scroll near boundary repeatedly; duplicate/empty/cyclic cursors; first/tail errors | One request per cursor/query; deduped stable rows; finite no-progress behavior; local retry; end versus bound distinguished. |
| New rows/prepend/eviction while scrolled away, focused or expanded | Stable row/pixel anchor and retained focus/detail; no forced jump. Returning to the live edge resumes log following. |
| Scope revocation/reconnect while a continuation is delayed | Old data/callbacks cannot resurrect the previous registration or widen to global Jobs/Logs. |
| Generation queued/running/terminal; live disconnect; mutation admitted/uncertain | Correct authoritative precedence, partial-content rules and reconciliation; queued acceptance never becomes a false completion. |
| Narrow/wide, 1×/2×, reduced motion; loading and populated compact grids | Matched skeleton geometry, cool static base, motion fallback, semantic reading order, visible focus and no decorative border/gloss regression. |

Review actual time sequences in addition to selected PNG/tree output. Static mockups and capture generation cannot prove absence of flicker, memory growth or scroll jumps. Record implementation, unit/Runtime results, native/browser visual inspection and installed-app evidence separately according to [native verification](../../wiki/code/native-verification.md). The current documentation work establishes none of those runtime results.
