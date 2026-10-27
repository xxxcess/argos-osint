# React UI design and implementation

[Code route](index.md) · [Intel web contract](intel-web.md) · [OpenCode](opencode-v2.md)

Apply to React UI work in `apps/web-desktop`, `packages/ui`, `packages/intel` and UI-facing changes in `packages/api-client`. The web theme is the **Dark Digitized HUD + Glassmorphism** system below; the TUI keeps its own `theme.rs`, and web work preserves layout semantics and product contracts. This is the shared design baseline for creation, updates and reviews; task-specific design direction can change it deliberately. Rust terminal UI follows its own [design contract](tui-design-spec.md).

## Ownership and stack

| Concern | Authority |
| --- | --- |
| Shared palette and primitives | [packages/ui](../../../packages/ui/src/index.tsx): `colors`, `fonts`, `Label`, `Copy`, `Button`, `Panel`, `Notice`, `LoadingState`, `EmptyState` |
| Screens and responsive composition | [packages/intel](../../../packages/intel/src/index.tsx): Bulletin, Focus Brief, Topology |
| Remote state and connection | [packages/api-client](../../../packages/api-client/src/index.tsx): Context, TanStack Query v5, readiness and backend/instance keys |
| Platform settings | [web adapter](../../../apps/web-desktop/app/backend-settings.ts): persisted endpoint, hydration-safe snapshots and request IDs through `BackendSettings` |
| Browser shell and CSS | [app](../../../apps/web-desktop/app), especially [style.css](../../../apps/web-desktop/app/style.css) |
| Behavior and offline fixtures | [Intel web contract](intel-web.md), [browser tests](../../../tests/web/intel.spec.ts), [Playwright config](../../../playwright.config.ts) |

Use installed manifests and the app's [Next guidance](../../../apps/web-desktop/AGENTS.md) for version-specific APIs. Next exports static files; request-time SSR, runtime Next API routes and server actions cannot serve this app. React Compiler is enabled for the web app and its shared transpiled packages; keep it enabled. Expo/Tauri/native builds are later milestones, not verified targets. Keep the pinned dependency set; add libraries only when the task needs them and validate compatibility.

## React Native and Compiler baseline

- Shared components use React Native primitives and `StyleSheet`, with web-only CSS/platform services supplied by the app. [Next configuration](../../../apps/web-desktop/next.config.mjs) aliases `react-native` to the pinned React Native Web install and transpiles `@argos/ui`, `@argos/intel` and `@argos/api-client`. Preserve this path; do not replace shared controls with DOM-only components.
- React Compiler is required in the Next dev/build pipeline, using pinned `babel-plugin-react-compiler` 1.0.0 with React 19's compiler runtime. `panicThreshold: "all_errors"` surfaces detected compilation failures; preservation of existing memoization guarantees is explicit. Do not disable the compiler or add blanket `use no memo` directives to bypass failures. Let it handle ordinary memoization; keep manual memoization only for a demonstrated need or an existing contract.
- [ESLint](../../../eslint.config.mjs) registers the stable recommended Hooks/compiler rules; `pnpm lint` fails on warnings too. Render must stay pure, Hooks unconditional, props/state immutable, ref access outside render and effect dependencies complete. Fix violations rather than suppressing rules or hiding state updates in timers.
- `BackendProvider` receives app-owned `BackendSettings`; `useSyncExternalStore` provides consistent static/client snapshots. The web adapter owns browser storage, URL configuration and UUID generation. Persisted choices load after hydration; blocked storage still permits a session connection. Future native hosts supply their own adapter and compiler integration; a passing web build is not a native validation claim.

Compiler optimization is not a correctness proof. Typecheck, Hooks/compiler diagnostics, strict compiled builds and behavior/visual verification work together. When changing the compiler pipeline or dependency pins, verify actual shared-package transformations or emitted compiler memoization as well as browser behavior; a flag alone is not evidence.

## Visual language

**Dark Digitized HUD + Glassmorphism** for `apps/web-desktop`, `packages/ui` and `packages/intel`. The TUI keeps [theme.rs](../../../crates/argos-osint-bin/src/tui/theme.rs) ([TUI design](tui-design-spec.md#theme-data-and-interaction)); the web uses this system while preserving layout semantics (pane order, Focus Brief 24/48/28, tabs when narrow) and product contracts. Do not copy terminal cell measurements.

Authority: tokens, `glass`, `glow()`, `Tone`/`toneColor`, `StatusTag`, `Frame` and `Panel` in [packages/ui](../../../packages/ui/src/index.tsx); grid backdrop, glow focus ring and thin scrollbars in [style.css](../../../apps/web-desktop/app/style.css). Add or change values there, never as scattered hex in feature components.

| Token group | Use |
| --- | --- |
| `colors` slate base/surfaces, text/dim/muted | Page, panels, hierarchy of text |
| accent cyan `#22d3ee`, `accent2` purple, `accentSoft` | Active/selected/focus, links, secondary emphasis |
| success, warning amber, danger crimson | Severity tones via `Tone`/`toneColor` |
| `glass`, `glassStrong`, `glassBorder` | Translucent slate fills and muted borders |
| `fonts.mono` / `fonts.body` | Telemetry labels, nav, metadata / prose |

Principles:
- Translucent slate fills, muted borders and backdrop blur. Blur and other web-only style props are web-only: cast them at the web boundary so native/RN types stay valid, and keep a readable opaque-enough fill when blur is unsupported.
- Cyber accents with restrained glow: `glow()` on active, focused or critical elements only, never on every surface.
- Bracketed uppercase mono telemetry labels (e.g. `[ STATUS ]`) for headings/metadata via `StatusTag`/`Frame`/`Panel`; prose stays in body font at a comfortable width.
- Thin rules, high density, compact controls; responsive grid that wraps (md:flex-row-like) rather than shrinking text.
- Contrast meets WCAG AA for text and essential controls against the glass fill over the backdrop. Honor `prefers-reduced-motion`; no motion needed to understand state.

## UI/UX guidance

- **State clarity:** active, selected, focus, hover, disabled, loading, empty, error and offline are distinct. **Glow is never the sole state indicator**; pair it with text, fill, border weight, `aria-pressed`/role state or icon.
- **Density:** pack telemetry tightly, but keep touch targets usable and body text readable; one hierarchy per screen, panels only for meaningful groups.
- **Tone usage:** danger = failure/blocking, warning = degraded/partial/stale, accent = active/informational, success = verified/healthy. Use `Tone`/`toneColor`, include a text label, and do not use tone to imply corroboration or confidence the data does not carry.
- **Do:** reuse `Panel`/`Frame`/`StatusTag`; keep focus-visible glow ring; test 1440/768/390/320px; preserve error recovery actions.
- **Don't:** stack nested glass cards, blur large scrolling lists needlessly, use glow/purple as decoration, tiny mono for essential actions or long text, or change contracts for style.
- **Review checklist:** tokens come from `@argos/ui`; AA contrast on glass; non-color state cues; focus ring visible; reduced motion honored; blur cast web-only with fallback; no overflow at four widths; fixture PNGs inspected; contracts in [intel-web.md](intel-web.md) unchanged.

## Current web components

The shell uses a compact monospace ARGOS / INTEL wordmark, API connection status and Bulletin/Topology navigation. Bulletin uses a bounded reading width, labeled search, wrapping categories, scrollable retained days, a loaded-row count and flat article rows with thin separators. Focus Brief keeps its three-pane proportions and compact narrow-screen pane controls; section headings and dividers supply hierarchy without nested card decoration. Topology uses simple titled regions and recorded relationships. Connection and action failures use a warm text/edge treatment with an explicit recovery action.

`Button` has bordered/default and quiet variants; active selection uses accent fill/dark text and web `aria-pressed`. Hover/press use the selected surface, disabled controls remain unavailable, and browser focus uses an independent accent outline. `Panel` owns section-heading typography and spacing; `Notice` is an alert. Monospace is for navigation/metadata, sans-serif for headlines and article descriptions. No theme toggle, animation library or new remote-data ownership was introduced.

## Layout and interaction

- Use `View`, `Text`, `Pressable`, `Image` and `StyleSheet` for shared UI; browser-only markup/styles belong in the app or an explicit web adapter. Keep strings inside `Text`; conditional children of `View` must not leak `0` or empty strings (`condition ? <Text>…</Text> : null` when needed).
- Preserve shrink/wrap behavior (`minWidth: 0`, bounded media, flexible text). At 1440, 768, 390 and 320px, inspect long content, navigation and controls for clipping, overlap and horizontal page overflow. Change layout when content stops fitting, not by shrinking all text.
- Existing Focus Brief uses approximately 24/48/28 Extracted/Reader/Context columns when wide and tabs when narrow. Preserve access to errors and actions on either layout. Feed images remain left of text; missing, unsafe or failed images collapse cleanly. Exact sizes and behavior live in the [web contract](intel-web.md#feed-imagery).
- Give every control an accessible name and appropriate role/state through RN accessibility props or semantic HTML at web boundaries. Preserve logical Tab order and visible `:focus-visible`; verify Enter/Space activation, tab selection and focus when content changes. Do not add redundant keyboard handlers to controls that already supply them.
- Label inputs independently of placeholders; associate errors/help with the field, retain entered values after failures, and offer a concrete recovery action. Keep touch targets usable at narrow widths without overcrowding adjacent controls.
- Distinguish initial loading, background refresh, empty results, partial data, action pending, action failure and offline/reconnect states. Avoid blanking readable content during ordinary refetch. Disable duplicate pending submissions and expose the result. Gate working UI/mutations on API readiness as required by the client contract.
- Reserve image dimensions/aspect ratio to avoid layout shifts; use meaningful accessible descriptions for informative images and avoid duplicate announcements for decorative thumbnails. Validate external URLs through existing helpers and preserve failure fallback. Honor reduced motion if adding animation; prefer transform/opacity and avoid motion required to understand state.

## Async data and pagination

`LoadingState` is an accessible RN indeterminate progress indicator with a label; `EmptyState` names a verified empty collection and its context. Pending days/feed/detail/evidence/Related/Topology requests do not render empty data claims. Background refresh keeps readable rows and shows a compact status; next-page requests have their own loading/error/retry state. Confidence zero is valid; unavailable scores remain unassessed.

One RN ScrollView owns workspace scrolling. Near the Bulletin's end, the client requests the next opaque cursor only when a page exists, no query fetch is running and the last next-page request is not in error. Query uses `cancelRefetch: false` to deduplicate repeated scroll events. Manual Load more/retry stays reachable. Loaded rows are deduplicated by run/article occurrence; category/day/search/backend/instance keys isolate scopes. Empty categories show a verified zero count; no retained days prompt Atlas collection.

## Focus Brief column order

Extracted contains confidence, actors and claims. Inferences are the last section of the middle Reader pane, after the article description and original-source link; narrow layouts keep them in the Reader tab. Preserve their distinct heading and pending/empty/error states. Core TUI places Inferences after Related at the end of its center stack. See [column order](../../diagrams/intel-focus-layout.html).

## Product meaning

Keep category/day/search, article occurrence identity, source attribution and topology seeds intact. Distinguish claims from inferences, unknown confidence from zero, and publication from indexing. Related leads are not corroboration. Reader currently displays the stored description; retained body availability does not authorize changing that behavior. Operational Jobs/Logs remain in Rust. See the [web contract](intel-web.md) before changing these semantics.

## Verification and evidence

Select the pinned Node with `nvm use` (load nvm in the shell if necessary); use the root pinned pnpm. For frontend code changes run `pnpm typecheck`, `pnpm lint` (including Hooks/compiler diagnostics, zero warnings), `pnpm build` (compiler enabled), `pnpm check:web-build`, then relevant offline `pnpm test:e2e` cases against the export. For compiler/dependency/launcher changes, also run `ARGOS_WEB_PROFILE=dev pnpm test:e2e` to force development compilation and check browser console/hydration behavior; static export success alone does not prove dev startup. Launcher changes additionally use the [process smoke](intel-web.md#verification). Respect the configured port/server ownership; preserve an existing user dev server. No Rust gate is needed for guidance-only changes; Rust changes follow the contributor workflow.

For visual/interaction changes, use the existing HTTP fixtures and a proportional state/viewport matrix. Capture affected screens with Playwright `page.screenshot({ path: …, fullPage: true })` into the named plan or `.agent-scratch/`, then **open and inspect the PNGs**. A screenshot file or passing E2E assertion is not visual approval. Check keyboard/focus, action outcomes and console/hydration errors separately. Add behavior assertions only for changed behavior or meaningful regressions. Keep fixtures offline; no live-provider data or secrets in evidence.

Report changed paths, checks actually run, inspected states/viewports and remaining gaps. Reviews list actionable `file:line` findings with impact and suggested correction; distinguish existing issues from regressions. If browser/image tooling is unavailable, record visual evidence as pending. OpenCode can run Playwright through its allowed shell without changing browser permissions; do not claim native-device verification from web captures.

## Sources and adaptation

Inspired by Vercel's [React Native skills](https://github.com/vercel-labs/agent-skills/tree/main/skills/react-native-skills), [React best practices](https://github.com/vercel-labs/agent-skills/tree/main/skills/react-best-practices), [composition patterns](https://github.com/vercel-labs/agent-skills/tree/main/skills/composition-patterns) and [web design guidelines](https://github.com/vercel-labs/agent-skills/tree/main/skills/web-design-guidelines), reviewed 2026-10-26. Compiler setup follows the [official React installation guide](https://react.dev/learn/react-compiler/installation), [Hooks/compiler lint reference](https://react.dev/reference/eslint-plugin-react-hooks) and [Next integration](https://nextjs.org/docs/app/api-reference/config/next-config-js/reactCompiler). These are original repo-specific instructions, not vendored upstream rule files. Retained ideas: accessible interaction, RN-safe rendering, explicit composition, focused state ownership and measured performance. Native-only packages/navigation, server caching/actions, Vercel deployment and wholesale upstream rule installation are outside this guidance. Routine work uses these local rules offline; refresh upstream deliberately when maintaining the skills.
