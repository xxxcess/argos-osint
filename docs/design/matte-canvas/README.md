# Glass canvas design reference

Approved **Glass Canvas** visual target for the native client and shared web paint (directory `docs/design/matte-canvas/` is retained temporarily for link stability). The four generated design references below use illustrative data and retain baseline information hierarchy; see [theme.md](theme.md) for the updated Glass Canvas material and desktop layout rules, and the [native verification guide](../../wiki/code/native-verification.md) for production-view captures.

## Read and reproduce

1. [Theme recipe](theme.md): exact **Glass Canvas** palette, smoked/frosted glass tiers, static liquid-metal rim highlights, crescent-moon action textures, opaque reduced-transparency fallbacks, and responsive desktop layout geometry.
2. [Live state and continuous scrolling](interaction-state.md): snapshot retention, local skeletons, opaque cursors, bounded rendering and stable scroll position.
3. [Logo and Quiet Orbit](motion.md): source assets, startup timing, proportional sizing, alpha handling and the settled mark.
4. [Implementation handoff](../../../.planning/2026-11-16-matte-canvas/handoff.md): historical baseline code edit map and dependency order.
5. [Reference manifest](reference-manifest.json): exact approved image identities, dimensions and provenance.

## Final material rules

| Surface | Required treatment |
| --- | --- |
| Brand display | Symbol only (fractured obsidian crescent + warm molten core, no orbit triangles, no visible wordmark) |
| Shell chrome (rail & top bar) | Smoked dark glass (`#0B0F14` alpha + subtle blur, `1px` hairline border & subtle liquid-metal rim) with warm molten-gold (`#E5B869`) active pill |
| Primary cards, Settings containers, metrics & readiness | Layered frosted/smoked dark glass (`#0B0F14`–`#121820` alpha + blur, crisp `1px` hairline border & static liquid-metal top rim); calm unboxed grids at rest that lift into brighter glass on hover/selection |
| Floating overlays (Inbox/Notifications popover, Command palette, Context menus, Modals) | Elevated frosted dark glass (`14–18px` same-window blur + crisp `1px` liquid-metal specular top rim + stacked card depth lips on grouped notifications) |
| Submit, Confirm, Create, Delete, Connect & brand hero accents | Fractured crescent-moon obsidian texture from the logo inside a tinted glass/obsidian button face with semantic tint and crisp liquid-metal top bevel |
| Routine controls (Cancel, filters, inputs, dropdowns, segmented pills, read retry) | Clean translucent smoked glass with crisp hairline/metallic borders rather than heavy texture noise |
| Reduced transparency fallback | Deterministic opaque `#0B0F14` / `#121820` / `#19212B` with identical bounds and crisp `#647587` borders under `ViewPreferences::matte()` / `prefers-reduced-transparency` |
| Initial unresolved content or pending list tail | Steady skeleton base with a restrained translated cool sheen |
| Long record lists | Continuous scrolling over bounded, opaque cursor requests; no routine page-number or Previous/Next chrome |

The deep-space backdrop is `#060709`. Build primary surfaces, chrome, cards, controls, and floating overlays as **layered smoked/frosted glass** with static liquid-metal hairline rim highlights, warm molten-core active accents, cyan interactive cues, and fractured crescent-moon textures derived from the logo. Keep desktop chrome calm and structured (slim left rail, empty native toolbar title area, compact top-right telemetry & bell-badge strip, hairline-grouped glass settings rows, floating notification card stack). Preserve accessible ≥ 4.5:1 text contrast, crisp keyboard focus and essential input boundaries, and avoid resting animation wake loops or pointer-lighting loops.

## Approved screens

### Workstation Status

![Baseline workstation overview layout](mockups/01-workstation-status.png)

The production Workstation takes its material cues from this reference. Use one compact registration band and three sections for Services, Status checks and Diagnostics. Keep actor search and scoped job activity easy to scan, with health and observation freshness displayed distinctly.

### Setup Gateway

![Baseline setup layout with symbol-only brand and textured Connect action](mockups/02-setup-gateway.png)

The logo anchors the crescent-and-molten-core palette. Connect receives crescent rock texture and a crisp liquid-metal top bevel because it submits the form. Navigation and routine utility controls use clean smoked glass. The stepper is informational; operator approval and credential boundaries are unchanged.

### Jobs & Logs

![Baseline continuous jobs list with local loading placeholders](mockups/03-jobs-logs.png)

Resolved rows and selection remain visible. The list tail and unresolved selected detail have independent loading regions. No Previous/Next or numbered page controls appear. State labels and icons reinforce color.

### Compact desktop

![Baseline compact overview layout](mockups/04-compact-workstation.png)

The same groups reflow into measured columns, with a compact navigation rail. This is a desktop layout concept, not an additional mobile app. Keep Ready and Stale distinct when both apply.

## Precedence and scope

The **2026-11-20 Glass Canvas** direction (deep-space `#060709` backdrop, smoked/frosted glass across primary surfaces and overlays, static liquid-metal rim highlights, crescent-moon textures, and the reference desktop layout) supersedes the 2026-11-16 matte-canvas surface restriction as well as earlier pager chrome and pointer-lighting loops. The [native control contract](../../wiki/code/native-design-system.md), [connection authority](../../wiki/code/native-connections.md) and [web product semantics](../../wiki/code/intel-web.md) still govern capabilities and data access. The scroll/state contract changes presentation without broadening authorization or server page bounds.

Earlier supplied references and dated implementation evidence remain historical. Do not reinterpret their screenshots or PASS records as evidence that this target has shipped. Future implementation must compare actual production-view captures with these references and report interaction, accessibility, performance and platform evidence separately.
