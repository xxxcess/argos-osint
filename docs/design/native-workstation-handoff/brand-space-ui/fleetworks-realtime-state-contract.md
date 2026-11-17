# Fleetworks realtime state and loading contract

> **Status in this repository.** The contract below is the owner's. The native parts implemented on this branch are: Jobs & Logs continuous cursor loading, readiness retained as stale on a failed refresh, Logs follow and older reads keeping a resolved status, a stable Logs status strip, and a top bar whose connection pill never swaps for request activity. Web `Skeleton.tsx` pulse, Intel Web states and the remaining native sections are not yet changed; see the [reproduction table](#reproduce) for what is covered and how to check each state. The matte theme replaces the "polished/liquid-glass" wording: placeholders use flat tonal steps and a cool shimmer only while loading.


## Outcome and baseline

Eliminate request-driven text flicker across native and web section components. Keep headings, readable content and interaction geometry stable while realtime work resolves. Use layout-matched skeleton shimmer for unresolved regions and concise, truthful local states. Preserve the same deep-space, liquid-glass material quality from splash through gateway selection and service screens, including loading, empty, stale and live states.

Baseline: Argos OSINT `nightly`, commit `00b700afd77ddac6cba2e2bd7ae91ab3802b7197`, tree `5051474185a8eb14b38e607480bb24c65352fff0`; audited **2026-11-16 America/New_York**. This is a mandatory companion implementation contract. The audits were read-only; no runtime reproduction, product tests or native/browser visual verification were performed. Verify the baseline and listed symbols before implementation; inspect relevant changes if HEAD moved. Preserve unrelated work.

## Audited mechanisms → required corrections

- Native `app.rs::view` replaces Online/Offline with **Syncing** through heterogeneous `any_busy` flags. `context_rail` maps polling activity to **Working/Ready**. Keep connection and domain status independent of transient requests.
- `State::complete_readiness` discards last-good readiness on refresh failure; `readiness_cards::checks` then replaces cards with an error strip and, on retry, skeletons. Retain usable same-scope checks with explicit stale status and existing action restrictions.
- `logs.rs::LogState::ticket` sets Loading even for Follow. `backend_views::jobs` uses `rows.is_empty() && busy()` for skeletons, so a verified empty result can return to an initial-loading presentation on every follow. Its `logs-loading`/`logs-synced` branches also change keys and wording. Preserve resolved empty and a stable live-status slot.
- `ui_components::async_status_strip` adds/removes a progress row. Brain edits render Pending debounce text, then Loading skeleton, then results. Preserve geometry; use one unresolved search presentation through debounce/fetch.
- Web `Skeleton.tsx` explicitly loops opacity 0.55→1→0.55; `LoadingState` text itself has no timer. Replace the skeleton pulse and fix consumer state/mount transitions. A CSS-only change cannot remove that JavaScript animation.

These are source-backed mechanisms, not claims of reproduced frame-level flicker. Preserve working retention in native host metrics, cycle history, Jobs polls and matching job detail, plus web cached refresh and terminal completion after live disconnect.

## Shared presentation-state contract

Derive section presentation from **resource identity + accepted snapshot + request activity + authoritative domain state + transport/freshness + mutation state**. Keep the existing controllers as authority; do not add duplicate polling, subscriptions or a competing cache. Commit related content, version and status coherently.

| State | Required presentation |
| --- | --- |
| Initial unresolved | No accepted snapshot for this identity. Keep heading, known labels and controls; skeleton only unresolved slots. Pending debounce and first fetch share this geometry. |
| Retained refresh | Keep the accepted snapshot, including a successfully empty one, through a same-identity read. Preserve rows, focus, selection, expansion and scroll. Use a quiet reserved indicator; never restore initial skeletons. |
| Authoritative generation | Queued/running/retry belongs to the active generation, regardless of fetch flags. Preserve existing web `StageContent` stage-local skeleton precedence over older output where required. Retain available independent content, including existing actor chips during body cleanup; expose partial output only where the current domain contract permits it. Failed/cancelled/superseded/blocked stages show diagnostics, not permanent skeletons. |
| Mutation pending | Mark the affected action/field busy; retain entered values and unrelated content. Preserve control size. Accepted/Queued is not Completed; preserve uncertain-outcome reconciliation and explicit optimistic rollback if used. |
| Settled data / empty | Commit accepted data immediately. Empty requires a successful authoritative result; repeated empty follows stay empty. Unknown/unavailable remains `—`; a valid zero remains `0` with its unit. |
| Stale / failed refresh | Keep usable same-scope content and its last successful timestamp, with compact **Stale** or **Update failed** plus recovery/details. An initial failure ends shimmer and shows a bounded error state. |
| Invalidated / blocked | Clear unusable or wrong-authority content. Show the appropriate reconnect/access/scope action. Retention must never expose another backend, registration, workstation, query or selected entity. |

Preserve native epoch/revision and binding/generation/sequence fences, web query namespaces, cancellation, expected-context revision and active-job validation. Only accepted completions may settle presentation: an obsolete response must not clear a newer busy state, erase its error or mark its generation complete. Terminal authoritative completion survives an unrelated live disconnect; active interrupted work retains truthful interruption status.

Render results/errors in the next committed update. No minimum loader duration, forced animation completion or cosmetic completion delay. Changes already available before the same paint may coalesce. Ordinary request boundaries must not rename sections or restart entry effects.

## Skeletons, copy, material and responsive layout

Use a constant-opacity, steady skeleton base with one broad dim translated sheen, approximately 1.2–1.8 seconds per cycle, clipped to placeholder geometry and driven by a shared phase within a visible section. Do not pulse whole-section opacity or sweep highlights across readable text. Stop hidden decorative animation. Reduced motion uses static placeholders with identical dimensions. Native matte rendering must not automatically prohibit an opaque shimmer when motion is allowed.

Use the companion theme's shared surface, border, depth and semantic-status tokens for placeholders and live indicators. Loading must not replace polished gateway/service cards with unrelated flat banners or saturated warning-like decoration. Keep status understandable through text/icon as well as color. Exclude skeleton shapes from accessibility navigation and announce meaningful accepted transitions once, not every poll.

Keep short headings such as **Jobs**, **Logs**, **Connection**, **Output**, **Confidence**, **Actors**, **Claims**, **Inferences** and **Related leads**. Use one local state, such as **Running**, **Stale** or **Live**, only when informative. Preserve “Related leads · not corroboration,” provenance, accessible names and actionable error detail. Remove transport narration such as “Debouncing catalog lookup…” from ordinary section copy. Reserve the status slot for localized labels; align changing numbers without moving adjacent controls.

Use responsive grids for dense overview/options/status groups, preserving semantic reading order and stable card/entity IDs. Derive placement from available width, not changing live values; do not reorder cards per poll. Reuse native measured row/column composition and web `Panel`/`Frame` containers. Keep tables for comparisons, Bulletin's record list and Brief's reading order. Native settled metrics stack below 540 logical pixels, but `skeleton_metric_tiles` currently always uses a row: make both share the same decision. Retain Brain's 720-pixel and Brief's 900-pixel composition constraints. Match skeleton padding, columns, row heights, media ratios and reserved content bounds to resolved layouts.

## Ordered edit map

Paths below are repository-relative; native paths share `apps/fleetworks-desktop/src/`.

| Order / location | Concrete work |
| --- | --- |
| 1. Native `app.rs`, `jobs_query.rs`, `jobs_effects.rs`, `logs.rs`, `log_effects.rs`, `workstation_dashboard.rs`, `brain.rs` | Derive the shared presentation contract from existing controllers. Preserve fences/retention; retain readiness on permitted transient failure; carry Jobs' retained-poll error into visible stale state. |
| 2. Native `ui_components.rs`, `material.rs`, `accessibility.rs`, `views.rs`, `readiness_cards.rs`, `backend_views.rs` | Stable section/status slots, concise copy, corrected empty-log classification, layout-matched skeletons and motion policy. |
| 3. `packages/ui/src/Skeleton.tsx`, `index.tsx::LoadingState`; `apps/web-desktop/app/style.css` | Remove recurring opacity pulse; implement shared steady-base shimmer and motion fallback without breaking density/accessibility APIs. |
| 4. `packages/intel/src/shell/JobStatusBeacon.tsx`, `packages/intel/src/shell/jobState.ts`, `packages/intel/src/shell/IntelShell.tsx`, `packages/intel/src/index.tsx` | Apply stable local status and preserve domain precedence, metric semantics and unchanged nodes. |
| 5. Under `packages/intel/src/views/`: `bulletin/BulletinView.tsx`, `bulletin/FeedHeader.tsx`, `brief/StageContent.tsx`, `brief/BriefProgressStepper.tsx`, `brief/ActorChips.tsx`, `brief/BriefSection.tsx`, `brief/BriefView.tsx`, `brief/ConfidenceSection.tsx`, `brief/ClaimList.tsx`, `brief/InferenceList.tsx` | Retain generation and operation/actor identity rules, tighten headings, and prevent unchanged updates from restarting section effects. |

## Controlled regression sequences and acceptance

Extend relevant existing tests: native `log_tests.rs`, `jobs_query_tests.rs`, `tests.rs`, `maintenance/tests.rs`, and inline `views.rs` / `workstation_dashboard.rs` tests. Under `packages/ui/__tests__/`, use `Skeleton.test.tsx`, `LoadingState.test.tsx` and `density-modes.test.tsx`. Under `packages/intel/__tests__/`, use `Bulletin.test.tsx`, `BriefView.test.tsx`, `BriefProgressStepper.test.tsx`, `IntelShell.test.tsx`, `revamp.test.tsx`, `useIntelQuery.test.tsx` and `live-query.test.tsx`.

Exercise: first load→success/empty; repeated unchanged retained refresh; empty log success→multiple empty follows; readiness refresh failure→retry→recovery; rapid query replacement→late old reply; overlapping generations→partial output→terminal result; mutation acceptance→actual completion; disconnect after completion; revoked/changed scope; unknown versus zero. Assert stable keys, geometry, focus, input, scroll/expansion, truthful states, and immediate completion/error resolution. Log tail-follow must respect a user who scrolls away.

Inspect actual temporal sequences, not just static screenshots: native small/large windows and DPI; web 1440/768/390/320 widths; both motion preferences. Use authorized focused checks and the shared production native Runtime/fixtures, then required repository gates. Report actual results and blockers separately from this specification.

## Reproduce

### State machine

One section has one presentation at a time. The controller, never a fetch flag, decides it.

| State | Enter | Shows | Leaves on |
|---|---|---|---|
| Initial unresolved | No accepted result for this scope | Heading, labels, layout-matched skeleton | First accepted reply (data, empty or error) |
| Settled | Accepted reply | Rows or a resolved empty message | New scope, refresh |
| Refresh | Background read of the same scope | **Same rows**; one small local activity indicator | Accepted reply or failure |
| Pending action | User started a mutation | The affected control is busy; its size does not change; other content stays | Accepted outcome |
| Stale / update failed | Transient failure with same-scope data | Last data, an `Update failed` label, a Retry control | Retry success, scope change |
| Failed (initial) | Failure with no data | Bounded error and a recovery action | Retry |
| Invalidated | Backend, registration, workstation, filter or query change | Cleared content, reconnect or scope action | New read |

Three rules apply everywhere: an outdated reply (older sequence, generation or binding) is dropped and never overwrites newer state; stale data is never kept across a backend or scope switch; completion and failure render in the next frame with no minimum loader time.

### What each native section does now

| Section | Behavior | Code | Test that reproduces it |
|---|---|---|---|
| Jobs list | Head page refreshes in place by poll. Rows and selection stay on fetch. `More` appends the next page at the tail. At most 64 pages and 500 rows. Duplicates dropped by job id. Failure keeps rows, shows `Could not load more` and Retry. Filter, scope or permission change clears head and tail. | `jobs_query.rs` (`JobsEvent::More`, `rows`, `can_load_more`, `at_bound`), `jobs_effects.rs`, `backend_views.rs::jobs_tail` | `jobs_query_tests.rs`: `tail_pages_append_without_duplicates_and_keep_the_head_order`, `row_cap_stops_loading_even_when_the_server_has_more`, `tail_failure_keeps_every_row_and_retry_recovers`, `an_outdated_tail_reply_never_overwrites_newer_state`, `a_scope_switch_clears_loaded_rows_and_ignores_the_old_tail_reply`, `a_head_poll_refreshes_the_head_and_keeps_appended_rows`, `a_failed_poll_keeps_same_scope_rows_and_a_selection_survives_load_more`, `load_more_appends_at_the_tail_while_rows_stay_visible` |
| Readiness checks | A failed refresh keeps the last checks of the same deployment, marks every check stale (so repairs stay restricted), shows `Update failed` and Retry. A reply for another deployment is an error and keeps nothing. | `app.rs::complete_readiness`, `readiness_cards.rs` | `readiness_cards.rs` tests, `maintenance/tests.rs::delayed_readiness_reply_controls_reindex_confirmation_and_failure` |
| Logs | Only a fresh refresh is `Loading`. Follow, older and export keep the resolved status, so loaded rows and a verified empty result never return to a skeleton. One status strip, one key (`logs-status`); its title follows the live toggle and job state, not each request. | `logs.rs::ticket`, `backend_views.rs` | `log_tests.rs::only_a_fresh_read_is_unresolved_while_follow_and_older_keep_the_resolved_status` |
| Top bar | The connection pill is always shown (Online, Connecting, Fault, Offline). Request activity is a spinner in a fixed 13 pt slot (`activity-slot`) beside it. | `app.rs` | Runtime tree: `activity-slot` present in every state |
| Skeletons | Flat tonal block, cool sheen only while motion is allowed; static under Reduced Motion. | `material.rs::skeleton_block` | `log_tests.rs::loading_list_areas_render_shape_accurate_shimmering_skeletons_with_reduced_motion_fallback` |

### Review each state by hand

1. Capture a registered fixture scene with the repository helper, for example `python3 scripts/fleetworks_verify.py native-ui --agent --scene <scene> --size 1280x800 --scale 1 --output .agent-scratch/native-ui/live-state`; list scene names with `native-ui --agent --list-scenes`. Jobs scenes are registered from `apps/fleetworks-desktop/examples/native/jobs_scenes.rs` (`filters_ready`, `page_two`, `no_matches`, `expired`, `permission_required`). See [native verification](../../../wiki/code/native-verification.md).
2. For stale data, complete a head poll with `Err(ClientError::Transport)` after a ready page. Expect the same rows, a short reason message and no skeleton.
3. For load-more failure, press **Load more** and complete the ticket with an error. Expect all rows, `Could not load more`, **Retry**; Retry reissues the same cursor.
4. For a scope switch, change the registration binding while a tail read is in flight. Expect an empty list and that the old reply is ignored.
5. For Reduced Motion, set `FixtureAccessibility { reduced_motion: true, .. }`. Expect static skeletons and no shimmer layers (`skeleton-shimmer`, `loading-shimmer` absent).
6. Keyboard: Tab reaches **Load more** after the last row; Enter or Space activates it; focus stays on the control while it shows `Loading more`.

Report which states you inspected as pixels and which only as tests; screenshots from before this change that show `Previous page`, `Next page` or `Page N` are obsolete.
