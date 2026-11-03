# React Native UI regression coverage

[React UI](react-ui.md) · [Web contract](intel-web.md) · [Contributor workflow](contributing.md)

`@tag:testing` — The shared React Native UI runs through React Native Web in the
offline Vitest and Playwright suites. Tests use real shared components and Query
providers; MSW or browser routes replace the API. Canvas unit tests replace only
the graph library, and browser tests exercise the actual canvas. No provider
credentials or user datastore are needed.

## Coverage map

| Surface | Regression contracts | Tests |
| --- | --- | --- |
| Shared primitives | Button activation/disabled/selected/focus/hover, readable prose and labels, panel/frame geometry, semantic status/error/empty states and density | `packages/ui/__tests__/*` |
| Error recovery | Failed child is contained, neighbors remain readable, Retry remounts recovered content, no inert Retry without a handler | `packages/ui/__tests__/ErrorBoundary.test.tsx` |
| Skeletons and motion | Decorative geometry stays out of accessibility tree, reduced-motion changes stop animation, delayed readiness cannot start animation after unmount | `packages/ui/__tests__/Skeleton.test.tsx`, Intel `revamp`, `landing*` tests |
| Home/readiness | Offline/connecting/ready/retry, readiness checks never invent success, Enter/Space launcher activation, retained Bulletin filters through Home | `HomeView`, `landing*`, web `App` tests; `tests/web/intel.spec.ts` |
| Bulletin | Search/debounce, category keyboard selection, date filter, pending vs verified empty/error, opaque cursors, deduplication, failed-page retry, scope changes, cached rows during refresh | `Bulletin`, `ServerAll`, `useIntelQuery` tests; browser `intel` contracts |
| Article cards/source | Whole-row actions, real attribution, no synthetic metadata, missing/unsafe/failed image collapse, image aspect ratio, long text at narrow widths | `ArticleCard`, `BriefView` tests; browser `intel` contracts |
| Focus Brief | Actor/claim/inference order, zero vs unavailable confidence, explicit empty output, retained data during refresh, actor enrichment independent of stored insights, topology seed selection, completed briefs do not auto-admit extraction | `BriefView`, `revamp`, `live-query` tests; browser `intel` and `brief-layout` contracts |
| Extraction stages | Queued/running/retry skeleton; failed/blocked/cancelled/superseded diagnostic; completed content/empty; independent claims/inferences/links; unrelated generations cannot replace current sections | `revamp`, `BriefView` tests; `tests/web/brief-layout.spec.ts` |
| Topology | No seed/loading/empty/failure/retry, selection/list/details sync, kind/depth filters, focus history, bounded graph notices, old seed cannot overwrite new graph | `topology`, `neighborhood` tests; browser `intel` contracts |
| Graph renderer | Input data copied before d3 mutation, escaping, labels/glyphs, selection, camera fitting/zoom retention, reduced motion, focused/cleared edges | `topology`, `cameraFrame`, `labelLayout`, `neighborhood` tests; browser graph interactions |
| Scroll and shell | Nodes/Connections scroll independently, content stays bounded, bottom navigation remains anchored, narrow layouts preserve navigation/API, network value/units share a row | `IntelShell` tests; browser `brief-layout`, `intel` and screenshot-gate contracts |
| Background jobs/metrics | Running/queued/failed/stale glow, failed work outranks successful work, glow clears on recovery, keyboard menu, zero/unavailable host metrics, chipset label | `IntelShell`, `revamp`, `live-query` tests; browser metrics/stage contracts |
| Navigation/history | Native navigator isolation and subscriptions, stable SSR snapshot, encoded identifiers, rejected structured/oversized seeds, topology return context, browser history/reload | web `navigation` tests; browser route/history contracts |
| Query/live boundaries | Readiness gating, error status/message, retained data after 408, server-switch isolation, selected-brief invalidation and deduplication, no REST polling for live data | `useIntelQuery`, `live-query`, web `live-transport` tests |
| Live transport | One same-origin socket, unsubscribe/reconnect/backoff, old-article/old-socket events ignored, heartbeat liveness, silent socket recovery, cleanup cancels retries | web `live-transport` tests |
| Wire validation | Nullable/zero metrics, bounded platform/chipset strings, stage identity and typed completion/refresh/indexing, rejected malformed events | `packages/api-client/__tests__/live.test.ts` |
| Compilation/hydration/visuals | Real Next React Compiler, wide/narrow hydration, runtime-error assertions, long content, screenshot geometry and intentional visual baselines | `tests/web/*.spec.ts`; CI production and development runs |

## Running and maintaining the suite

Ask before running verification unless the user has already authorized its scope.
`pnpm test` discovers the complete offline component, hook and adapter suite.
`pnpm typecheck` and `pnpm lint` are separate requirements. Vitest does not compile
components with React Compiler, so browser verification is needed after component
or state changes.

`ARGOS_WEB_PROFILE=dev pnpm test:e2e` starts an isolated development server using
the repository Playwright configuration. Development verification sets
`ARGOS_WEB_TEST_DEV=1`, giving Next its own output and lock directory under
`.agent-scratch/next-web-tests`; the user launcher retains `.next`. Custom test
configurations must preserve that environment setting. A different TCP port
alone does not isolate Next development servers. The production browser pass uses the
existing `pnpm build` then `pnpm test:e2e` workflow. CI runs both profiles; see
[CI guide](ci.md#intel-webyml--local-web-client-checks). Screenshots honor
`ARGOS_WEB_CAPTURE_DIR` so CI retains them with the test artifacts.

Screenshot-only fixtures use reduced motion. Dedicated sequence/motion tests
exercise timed transitions explicitly. Do not lengthen every screenshot timeout
to wait through the full launcher presentation. Keep deferred requests controlled
by a gate and release them in cleanup. A test of a disabled query must establish
that the backend is ready, so an unrelated offline gate cannot make it pass.

Chromium visual baselines are shared across hosts using a fixed locale, UTC clock, loaded fonts and reduced motion; developer chrome is excluded. Review changed screenshot baselines against the intended design and behavioral
assertions; never update them merely to silence a failure. Keep baseline platform
differences separate from functional failures.

## Limits

This coverage is for shared React Native components on the web host. It does not
establish iOS/Android device rendering, native accessibility services or GPU/driver
behavior on every platform. Native font-family branches are checked as pure
contracts, while actual native-host verification needs a native application and
device runner. Browser fixtures prove client handling of API responses; they do
not prove Rust persistence or extraction/model correctness. No test suite can
guarantee that future changes introduce no regressions.
