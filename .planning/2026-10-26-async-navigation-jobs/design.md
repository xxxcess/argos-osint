# Async UI and navigation update

## Baseline and scope
Built on9ff89fe. Preserve current core-TUI-themed React Native Web presentation, strict React Compiler/Hooks checks, API readiness gating and occurrence/cursor semantics. Native clients remain planned. This design records the requested change, not completed verification.

## React data presentation
Shared `LoadingState` uses an accessible indeterminate RN indicator and label; `EmptyState` uses a concise heading and useful next action/context. Initial loading, background refresh, next-page loading, read failure and verified empty results are distinct. Keep readable retained data during same-query refresh; do not show empty collections while their request is pending. Valid numeric0 (e.g. confidence) remains a value.

One RN ScrollView owns workspace scrolling; Bulletin fetches the next opaque cursor near the bottom, using Query's cancelRefetch:false deduplication and fetching/error guards. A manual Load more/retry remains available. Category/day/search/backend changes use distinct Query keys; loaded occurrences are deduplicated by run/article identity. No DOM scroll listener or new dependency. Apply state components to days/feed, detail evidence/context, Related, Topology and API readiness where relevant.

## Venture navigation and lifecycle
The no-argument CLI owns a menu/session loop. After a launched TUI/session completes and services/terminal guards are restored, it reopens Venture. Explicit `stack up` commands remain one-shot. Terminal prompts keep a validated answer/draft history: Esc returns to the previous question/value; Esc at the service root exits; Ctrl+Q/Ctrl+C and explicit cancel retain cancellation. Replaying earlier decisions never applies storage or launches children. Changing a branch invalidates incompatible later answers.

Each selected session runs in a child process; process exit releases its global repository and background workers before the parent opens the menu. PTY tests cover Escape backward through Web settings, edited value retention, TUI launch/quit/menu return and final root exit. Existing API/Web wizard process tests must explicitly exit the returned menu.

## Jobs filter freshness
On user status/app/search changes, clear pending owner navigation and invalidate the displayed rows/detail/selection/scroll before the asynchronous read. Show Loading jobs until the current snapshot arrives; same-filter refresh retains data. Reject stale module/filter keys and request the latest scope after an in-flight older read. Filter labels/action geometry participate in layout invalidation. Tests hold a delayed snapshot, trigger both mouse/keyboard filters and verify the next frame, then deliver an old/current snapshot and check final rows.

## Documentation and agent guidance
Root/scoped/OpenCode instructions point to docs/wiki/code/ci.md and affected .github workflows before choosing checks, respecting Code Architect ownership. README records the actual pinned Rust/SurrealDB/Ratatui/Tokio/Axum and Node/pnpm/React/RN/RNWeb/Next/Compiler/Query/TypeScript/Playwright/ESLint stack. Update usage, interface/component/design/data-responsiveness contracts, React/Intel contracts, CI guide and branch scope. Add a Venture navigation figure pair for return/back behavior and retain process ownership/readiness boundaries.

## Acceptance
Delayed/empty/zero/paginated browser fixtures at1440/768/390/320; duplicate scroll and category-switch isolation; compiler-enabled export/dev suites and PNG inspection. Jobs filter action/snapshot tests plus scoped normal/compact/loading/empty captures. Venture prompt replay tests, actual PTY return/back checks, and full embedded Rust gate after combined changes. Instructions/links/diagram parity validated; hosted/native/live-provider checks reported separately.

## Verification-discovered reader correction
The existing Intel overflow fixture assumed acquisition-body rendering. The actual reader contract is stored description; See more must use that same source. Share the source accessor, use distinct retained body and long description in the fixture, and recompute the brief stack position after changing text length. Keep all scroll/hit/focus/overlay assertions and inspect wide/compact reader captures.
