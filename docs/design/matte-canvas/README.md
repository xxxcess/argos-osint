# Matte canvas design reference

Approved visual target for the native client and shared web paint. The four generated design references below use illustrative data. See the [native verification guide](../../wiki/code/native-verification.md) for production-view captures.

## Read and reproduce

1. [Theme recipe](theme.md): exact palette, matte compositing, softly blended group edges, action-role texture rules, component states and responsive geometry.
2. [Live state and continuous scrolling](interaction-state.md): snapshot retention, local skeletons, opaque cursors, bounded rendering and stable scroll position.
3. [Logo and Quiet Orbit](motion.md): source assets, startup timing, proportional sizing, alpha handling and the settled mark.
4. [Implementation handoff](../../../.planning/2026-11-16-matte-canvas/handoff.md): verified code edit map, dependency order and future validation.
5. [Reference manifest](reference-manifest.json): exact approved image identities, dimensions and provenance.

## Final material rules

| Surface | Required treatment |
| --- | --- |
| Brand display | Symbol only (fractured obsidian crescent + warm molten core, no orbit triangles, no visible wordmark) |
| Floating overlays (Inbox/Notifications popover, Command palette, Context menus, Modals) | Frosted dark glass (same-window backdrop blur + crisp 1px liquid-metal specular top rim; opaque fallback under reduced transparency) |
| Submit, Confirm, Create, Delete, Connect & brand hero accents | Fractured crescent-moon obsidian texture from the logo with semantic tint and restrained liquid-metal top bevel |
| Navigation, Settings, Cancel, filters, refresh, pop-out and read retry | Clean deep-space obsidian/slate with subtle warm molten-gold or cool-metallic active pills; hairline dividers inside grouped containers |
| Metrics, readiness, cards, tables and selectable rows | Quiet deep-space tonal groups (unboxed grids at rest, soft glass/elevated lift on hover/selection); spacing and alignment establish hierarchy |
| Initial unresolved content or pending list tail | Steady skeleton base with a restrained translated cool sheen |
| Long record lists | Continuous scrolling over bounded, opaque cursor requests; no routine page-number or Previous/Next chrome |

The canvas is `#060709`. Pair deep-space obsidian surfaces with frosted-glass floating overlays, static liquid-metal hairline rim highlights, warm molten-core active accents, and fractured crescent-moon textures derived from the logo. Keep desktop chrome calm and structured (slim left rail, centered command trigger pill, compact top-right telemetry & bell-badge strip, hairline-grouped settings rows, floating notification card stack). Preserve accessible ≥ 4.5:1 text contrast, crisp keyboard focus and essential input boundaries, and avoid resting animation wake loops or pointer-lighting loops.

## Approved screens

### Workstation Status

![Approved smooth matte workstation overview](mockups/01-workstation-status.png)

One resources group, a compact client/gateway band and a readiness grid. The last verified health and freshness are separate. The context rail is omitted when it would duplicate visible information.

### Setup Gateway

![Approved setup with symbol-only brand and textured Connect action](mockups/02-setup-gateway.png)

The logo anchors the crescent-and-molten-core palette. Connect receives crescent rock texture and a subtle liquid-metal top bevel because it submits the form. Navigation and routine utility controls stay clean and uncluttered. The stepper is informational; operator approval and credential boundaries are unchanged.

### Jobs & Logs

![Approved continuous jobs list with local loading placeholders](mockups/03-jobs-logs.png)

Resolved rows and selection remain visible. The list tail and unresolved selected detail have independent loading regions. No Previous/Next or numbered page controls appear. State labels and icons reinforce color.

### Compact desktop

![Approved compact overview with blended surfaces](mockups/04-compact-workstation.png)

The same groups reflow into measured columns, with a compact navigation rail. This is a desktop layout concept, not an additional mobile app. Keep Ready and Stale distinct when both apply.

## Precedence and scope

The **2026-11-20** deep-space + frosted-glass + liquid-metal + crescent-texture direction and reference desktop layout refine the 2026-11-16 matte baseline and supersede earlier pager chrome, noisy pointer-lighting loops, and flat-only restrictions on floating overlays and brand highlights. The [native control contract](../../wiki/code/native-design-system.md), [connection authority](../../wiki/code/native-connections.md) and [web product semantics](../../wiki/code/intel-web.md) still govern capabilities and data access. The scroll/state contract changes presentation without broadening authorization or server page bounds.

Earlier supplied references and dated implementation evidence remain historical. Do not reinterpret their screenshots or PASS records as evidence that this target has shipped. Future implementation must compare actual production-view captures with these references and report interaction, accessibility, performance and platform evidence separately.
