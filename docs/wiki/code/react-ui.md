# React UI design and implementation

[Code route](index.md) · [Intel web contract](intel-web.md) · [OpenCode](opencode-v2.md)

Apply to React UI work in `apps/web-desktop`, `packages/ui`, `packages/intel` and UI-facing changes in `packages/api-client`. The web theme is the **Dark Digitized HUD + Glassmorphism** system below; the TUI keeps its own `theme.rs`, and web work preserves layout semantics and product contracts. This is the shared design baseline for creation, updates and reviews; task-specific design direction can change it deliberately. Rust terminal UI follows its own [design contract](tui-design-spec.md).

## Argos Look & Feel (reference)

Direction is the Argos OSINT terminal desk — professional, sparse, evidence-first — plus the HUD/glass treatment below. Stay inside that theme. Component layout, grouping, and treatment may be original when they use the shared tokens, type, accent, and density. A generic card-on-white layout, a new palette, or a second brand is out of theme. Name the screen type and density (default or compact) before building. Adapted from the [frontend design skill](https://codegen.com/skills/frontend-design/skill.md); this baseline decides the theme, and component design may vary inside it.

1. **Direction.** Design on the terminal desk. Change the theme only when the user asks.
2. **Type.** `fonts.mono` for telemetry, nav, and metadata; `fonts.body` for prose. Hold the existing scale. No extra face.
3. **Space.** One base unit (~8–12px). Derive padding and gaps from it. Default lists and cards keep a visible gap. Compact or zero-gap only for launchers, command palettes, headline lists, and category bullets.
4. **Color.** Neutral surfaces, one accent for titles, primary actions, active navigation, and focus. Select is a separate token. Semantic colors only for state. No decorative gradient.
5. **Components.** Accent titles and thin frames are the theme signature. Compose or add components freely inside that signature: new groupings, frames, meters, and stripes are fine when they carry information. Extend a shared primitive first; add one when the screen needs a form the set does not have. Bold key values at normal size.
6. **States.** Hover, focus, active, selected, disabled, loading, empty, and error are designed with the layout. Contrast and focus order come first. Motion stays light; `prefers-reduced-motion` must still show state. Use real labels and data, not placeholder copy.

Scroll long content; do not squeeze charts, tables, or critical rows. Ellipsize labels. React Native topology graphs use [ForceCanvas.tsx](../../../packages/intel/src/topology/ForceCanvas.tsx) (`react-force-graph-3d`) plus the accessible list twin. Do not add another graph library. The TUI keeps its existing topology view and does not gain new chrome, motion, or density modes.

| Principle | React Native | TUI (keep simple) |
| --- | --- | --- |
| Accent hierarchy | Titles, primary buttons, focus rings | ACCENT titles + focus border + `• focused` |
| Thin separation | Subtle border or light elevation, one padding unit | Thin borders, 1-cell padding |
| Controlled density | Spaced by default; compact only for launchers/palettes/headlines | Same rule; no new modes |
| Focus vs selection | Distinct states; focus never hides other content | ACCENT/bold = activation; SELECT = keyboard focus |
| Scroll over squeeze | Min sizes + scroll; ellipsize labels | Charts ≥11 rows, tables ≥9; scroll |
| Theme, with room to compose | Tokens for color, type, space, and radius; component layout may be original | `theme.rs` remains authoritative |
| Topology graphs | `react-force-graph-3d` via `ForceCanvas` | Existing canvas; no force-graph port |

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

**TUI palette + HUD/glass treatment** (Dark Digitized HUD + Glassmorphism) for `apps/web-desktop`, `packages/ui` and `packages/intel`. The palette is the core TUI palette in [theme.rs](../../../crates/argos-osint-bin/src/tui/theme.rs) ([TUI design](tui-design-spec.md#theme-data-and-interaction)); the web keeps those colors and adds the HUD/glass treatment while preserving layout semantics (pane order, Focus Brief 24/48/28, tabs when narrow) and product contracts. Do not copy terminal cell measurements.

Authority: tokens, `glass`, `glow()`, `Tone`/`toneColor`, `StatusTag`, `Frame` and `Panel` in [packages/ui](../../../packages/ui/src/index.tsx); grid backdrop, glow focus ring and thin scrollbars in [style.css](../../../apps/web-desktop/app/style.css). Add or change values there, never as scattered hex in feature components.

| Token | Value | Use |
| --- | --- | --- |
| bg / panel / raised | `#141414` / `#1e1e1e` / `#303030` | Page, panels, selected/raised rows |
| border | `#535353` | Thin rules and borders |
| text / dim / muted | `#e8e8e8` / `#b5b5b5` / `#919191` | Primary, secondary, quiet metadata |
| accent (sky) / accent2 (violet) | `#87bfff` / `#c678dd` | Active/focus/links / secondary emphasis |
| success / warning / danger | `#98c379` / `#e5c07b` / `#f08a8a` | Severity tones via `Tone`/`toneColor` |
| `glass`, `glassStrong`, `glassBorder`, `accentSoft` | derived from TUI surface grays | Translucent fills, muted borders, soft accent wash |
| `fonts.mono` / `fonts.body` | | Telemetry labels, nav, metadata / prose |

Values mirror `theme.rs`; recheck it when changing the palette.

Principles:
- Translucent slate fills, muted borders and backdrop blur. Blur and other web-only style props are web-only: cast them at the web boundary so native/RN types stay valid, and keep a readable opaque-enough fill when blur is unsupported.
- Cyber accents with restrained glow: `glow()` on active, focused or critical elements only, never on every surface.
- Bracketed uppercase mono telemetry labels (e.g. `[ STATUS ]`) for headings/metadata via `StatusTag`/`Frame`/`Panel`; prose stays in body font at a comfortable width.
- Thin rules and consistent padding. Default list and card spacing stays comfortable; compact or zero-gap only for launchers, command palettes, headline lists, and category bullets. Responsive grid wraps (md:flex-row-like) rather than shrinking text.
- Contrast meets WCAG AA for text and essential controls against the glass fill over the backdrop. Honor `prefers-reduced-motion`; no motion needed to understand state.

## UI/UX guidance

- **State clarity:** active, selected, focus, hover, disabled, loading, empty, error and offline are distinct. **Glow is never the sole state indicator**; pair it with text, fill, border weight, `aria-pressed`/role state or icon.
- **Density:** default spacing leaves a visible gap between records. Compact spacing is only for launchers, command palettes, headline lists, and category bullets. Keep touch targets usable and body text readable; one hierarchy per screen, panels only for meaningful groups.
- **Tone usage:** danger = failure/blocking, warning = degraded/partial/stale, accent = active/informational, success = verified/healthy. Use `Tone`/`toneColor`, include a text label, and do not use tone to imply corroboration or confidence the data does not carry.
- **Do:** reuse `Panel`/`Frame`/`StatusTag`; keep focus-visible glow ring; test 1440/768/390/320px; preserve error recovery actions.
- **Don't:** stack nested glass cards, blur large scrolling lists needlessly, use glow/purple as decoration, tiny mono for essential actions or long text, or change contracts for style.
- **Review checklist:** tokens come from `@argos/ui`; AA contrast on glass; non-color state cues; focus ring visible; reduced motion honored; blur cast web-only with fallback; no overflow at four widths; fixture PNGs inspected; contracts in [intel-web.md](intel-web.md) unchanged.

## Current web components

The shell uses a compact mono ARGOS / INTEL wordmark, API connection status and a bottom Desktop/Bulletin/Topology taskbar over the grid backdrop. Desktop presents a centered Intel launcher; Bulletin stays the default entry and retains its category/day/search state when switching views. The category rail sits beside the feed at 1000px and wider and wraps above it on narrower screens. Bulletin keeps a bounded reading width, labeled search, wrapping categories, scrollable retained days and a loaded-row count; article rows are glass cards with a left **tone stripe** and a `StatusTag`. Focus Brief keeps its three panes (tabs when narrow) as `Panel`s; Topology uses titled panels and recorded relationships. Connection and action failures use a danger-toned `Notice` with an explicit recovery action.

- `Button`: `default` is a framed neutral surface — `colors.border` over `colors.glass` — visually distinct from passive `Panel`/card glass; hover (`accentSoft` fill) is distinct from pressed (`glassStrong`) without motion, and `active`/`aria-pressed` uses accent fill with dark text. `quiet` keeps a persistent thin `colors.border` frame over transparent fill with dim text (no longer frameless). `primary` is the single decision action per surface — Connect/retry, Ensure/resume brief, Load more/Retry — with accent border, accent text, `accentSoft` fill and `glow(accent)` on hover. Disabled uses muted text (`colors.muted`), `glassBorder` border and `accessibilityState.disabled` rather than bare opacity. Selection and the focus-visible glow ring stay independent of each other.
- Related-lead rows are interactive body-font rows (`RelatedLead` `Pressable`, minHeight 40) with a mono basis line and `›` chevron, not uppercase mono `Button` text; narrow Focus Brief tab state is owned by `Button` `active`, not a wrapper glow. Topology node-list rows are pressable at a 40px target with hover (`glassStrong`) and a neutral left tick on press; selection keeps an accent left border plus `accentSoft`.
- `Panel`: glass container with bracketed uppercase mono heading. `Frame`: bracketed HUD frame for grouped telemetry. `StatusTag`: toned bracketed label (always carries text). `Notice`: alert. `Label`/`Copy`: mono metadata / body prose.
- Mono is for navigation/telemetry, body font for headlines and descriptions. No theme toggle, animation library or new remote-data ownership was introduced.

## Canvas visualizations

Canvas content is invisible to assistive tech and tests. Every canvas needs an **accessible list/text twin** that carries the same entities, relationships, selection and filters, plus a summary label on the canvas. React Native topology graphs use the force-graph canvas (`react-force-graph-3d` in `ForceCanvas`); do not replace it with another chart library or a hand-rolled layout. Pass tokens (`colors`, `toneColor`, palette) into draw code as props rather than hard-coding hex; keep kind/state distinguishable beyond color. Load canvas libraries lazily and client-only, hand them cloned data, and honor reduced motion (no animated transitions, no drag, pre-settled layout). Offer zoom/fit/reset/pause controls as real buttons. See the [topology contract](intel-web.md#interactive-topology-graph).

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

## React component testing (real-component pattern)

@tag:testing

Run `pnpm test` from the workspace root. [Vitest configuration](../../../vitest.config.ts) discovers package and app tests in jsdom and aliases React Native to React Native Web, as the web build does. [Setup](../../../vitest.setup.tsx) supplies the shared test environment. Use React Testing Library to render real components and MSW to intercept the HTTP boundary with offline fixtures.

- Assert user-visible loading, success, empty, error/retry and pending states. Isolate Query clients, handlers and settings between cases; preserve real query keys and request serialization.
- Await Testing Library/user-event helpers, which already wrap React updates. Use React `act` for direct DOM events or imperative updates that those helpers do not wrap.
- Compare rendered declarations against `@argos/ui` tokens. The [style helpers](../../../vitest.test-support.tsx) read RN Web atomic stylesheet declarations because jsdom cannot fully compute those styles. These assertions do not prove browser contrast, focus rings or visual layout.
- Check hover, press, focus and selected states against each component's actual contract. Default and compact density remain restricted by the design baseline above; do not invent component variants solely for a test.
- Use [offline browser tests](../../../tests/web) for real keyboard operation, responsive layout and screenshot comparison. Run `pnpm test:e2e` after the static build; review changed baselines deliberately and inspect affected PNGs at 1440, 768, 390 and 320px. Snapshot generation alone does not establish visual approval.

New or modified test source comments may use `@tag:testing` to connect this contract to executable checks. Keep test outcomes in task evidence, rather than recording volatile pass counts or flaky-test exceptions here.

For visual/interaction changes, use the existing HTTP fixtures and a proportional state/viewport matrix. Capture affected screens with Playwright `page.screenshot({ path: …, fullPage: true })` into the named plan or `.agent-scratch/`, then **open and inspect the PNGs**. A screenshot file or passing E2E assertion is not visual approval. Check keyboard/focus, action outcomes and console/hydration errors separately. Add behavior assertions only for changed behavior or meaningful regressions. Keep fixtures offline; no live-provider data or secrets in evidence.

Report changed paths, checks actually run, inspected states/viewports and remaining gaps. Reviews list actionable `file:line` findings with impact and suggested correction; distinguish existing issues from regressions. If browser/image tooling is unavailable, record visual evidence as pending. OpenCode can run Playwright through its allowed shell without changing browser permissions; do not claim native-device verification from web captures.

## Sources and adaptation

Inspired by Vercel's [React Native skills](https://github.com/vercel-labs/agent-skills/tree/main/skills/react-native-skills), [React best practices](https://github.com/vercel-labs/agent-skills/tree/main/skills/react-best-practices), [composition patterns](https://github.com/vercel-labs/agent-skills/tree/main/skills/composition-patterns) and [web design guidelines](https://github.com/vercel-labs/agent-skills/tree/main/skills/web-design-guidelines), reviewed 2026-10-26. Compiler setup follows the [official React installation guide](https://react.dev/learn/react-compiler/installation), [Hooks/compiler lint reference](https://react.dev/reference/eslint-plugin-react-hooks) and [Next integration](https://nextjs.org/docs/app/api-reference/config/next-config-js/reactCompiler). These are original repo-specific instructions, not vendored upstream rule files. The [frontend design skill](https://codegen.com/skills/frontend-design/skill.md) (reviewed 2026-10-28) supplies the decision order — direction, type, space, restrained color, a theme signature, designed states — and defers to this baseline. Component composition may vary inside the Argos theme. Retained ideas: accessible interaction, RN-safe rendering, explicit composition, focused state ownership and measured performance. Native-only packages/navigation, server caching/actions, Vercel deployment and wholesale upstream rule installation are outside this guidance. Routine work uses these local rules offline; refresh upstream deliberately when maintaining the skills.
