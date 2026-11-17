# Fleetworks matte deep-space theme

Companion to [the splash handoff](fleetworks-splash-handoff.md) and [the realtime state contract](fleetworks-realtime-state-contract.md). It replaces the owner's earlier "liquid-glass" theme draft. The owner's addendum (see [the overhaul spec](../../../specs/fleetworks-native-web-ui-overhaul.md#addendum-quiet-orbit-matte-theme-and-stable-live-states)) is the rule: **matte surfaces, sheen only on the main logo, a cool shimmer only while loading, fractured-obsidian texture only on real Submit, Confirm, Create, Delete and Connect actions.**

## 1. Rules

1. One uninterrupted canvas, `#060709`, from the splash through gateway selection and every service screen. No image-sized vignette, no pale fringe, no exposure jump.
2. Hierarchy comes from charcoal tonal steps, spacing and type weight, not boxes or heavy outlines. Panel edges blend into the canvas. Input edges, keyboard focus and contrast stay meaningful.
3. Text is cool gray. No ivory or gold primary accent on navigation, cards, fields, utility controls or the composer.
4. Permanent metallic sheen appears only on the main logo. A restrained cool shimmer appears only while something is loading (skeletons, busy controls) and never under Reduced Motion.
5. The fractured-obsidian texture appears only on actual Submit, Confirm, Create and Delete actions. **Connect counts as a submission.** Navigation, Settings, filters, toggles, refresh and other utilities are smooth and matte. Context color tints the action; it never makes a reflective metal.
6. A status color always travels with a label or an icon.
7. Dense overview, option and readiness screens use responsive grids: resources first, then client and gateway, then service readiness or exceptions. Keep tables where comparison matters. Card and label positions do not move as values update.

## 2. Tokens

Native tokens live in `apps/fleetworks-desktop/src/theme.rs`. Web mirrors them in `packages/ui/src/index.tsx` (`colors`), `packages/intel/src/shell/intel-theme.ts` (`intelColors`) and `apps/web-desktop/app/style.css`. `packages/ui/__tests__/palette-contrast.test.ts` reads `theme.rs` and fails when the two drift.

| Role | Native constant | Web token | Value |
|---|---|---|---|
| Canvas, inset wells | `BACKGROUND`, `SURFACE_INSET` | `colors.bg`, `colors.inset`, `intelColors.bg` | `#060709` |
| Step 1: content | `SURFACE` | `colors.panel`, `intelColors.panel` | `#0B0E13` |
| Step 2: raised | `SURFACE_RAISED` | `colors.raised`, `intelColors.panelRaised` | `#111720` |
| Step 3: elevated, hover | `SURFACE_ELEVATED` | n/a (native) | `#181E28` |
| Decorative edge | `BORDER_SUBTLE` | `colors.borderSubtle`, `intelColors.borderDecorative` | `#262D38` |
| Control edge (essential) | `BORDER_CONTROL` | `colors.border`, `intelColors.borderStrong` | `#6F7B8D` |
| Text | `TEXT_PRIMARY` | `colors.text` | `#E8EDF4` |
| Supporting text | `TEXT_SECONDARY` | `colors.dim` | `#BEC7D4` |
| Muted text | `TEXT_MUTED` | `colors.muted`, `intelColors.faint` | `#9AA6B6` |
| Neutral accent (selection, links) | `ACCENT` | `colors.accent` | `#A9B8CC` |
| Accent pressed / hover / focus / deep | `ACCENT_PRESSED`, `ACCENT_HOVER`, `ACCENT_HIGHLIGHT`, `ACCENT_DEEP` | `accentPressed`, `accentHover`, n/a, n/a | `#8FA1B8`, `#C3CFDF`, `#DCE6F3`, `#4F5E73` |
| Ink on accent | `ON_ACCENT` | `colors.onAccent` | `#060709` |
| Selected surface | `SELECTION_SURFACE` | `colors.selectionSurface` | `#1B2533` |

### Status colors

| Meaning | Native | Web | Value | Cue to pair with it |
|---|---|---|---|---|
| Live, running | `StatusTone::Cyan` (`ENTITY_TECH`) | `intelStatus.running` | `#21D9EE` | `Live` or `Running` label, spinner |
| Info, measurement | `StatusTone::Info` (`STATUS_INFO`) | `intelColors.cool` | `#5AB8F0` | unit or info icon |
| Healthy, complete | `StatusTone::Ok` (`STATUS_OK`) | `intelColors.success` | `#5FD39A` | check or `Ready` label |
| Warning, stale | `StatusTone::Warn` (`STATUS_WARN`) | `intelColors.warning`, `intelStatus.stale` | `#E5B45B` | warning shape and short cause, age for stale |
| Error, destructive | `StatusTone::Err` (`STATUS_ERR`) | `intelColors.danger` | `#F06A8A` | failure icon or destructive verb |
| Queued, paused | `StatusTone::Violet` (`ENTITY_GRAPH`) | `intelStatus.queued` | `#B48EFF` | clock or pause icon, `Queued` label |
| Unknown, unavailable | `StatusTone::Neutral` | `intelStatus.idle` | `#9AA6B6` | explicit `Unavailable` or `—` |

Unknown is never zero. A valid zero keeps its unit. Gauge bands need real thresholds.

## 3. Where each rule lives

| Concern | Native | Web |
|---|---|---|
| Tokens | `theme.rs` | `packages/ui/src/index.tsx`, `intel-theme.ts`, `apps/web-desktop/app/style.css` |
| Flat cards and tiles | `material.rs::luminous_card`, `interactive_tile` (fill, 1 px edge, hover step; no gradient, glow or shadow) | `matteCard`, `matteDarkControl` in `packages/ui/src/index.tsx` |
| Kit depth tokens | `material.rs::configure_kit` (edges only; no drop shadow) | n/a |
| Control finishes | `material.rs::MetalFinish` and `apply_liquid_metal` (the function name is historical) | `Button` variants in `packages/ui/src/index.tsx` |
| Fractured texture | `MetalFinish::Fractured`, `FracturedDanger`, `material.rs::fracture_shape` | not yet drawn on web; Submit and Confirm use the flat primary and danger fills |
| Loading shimmer | `material.rs::skeleton_block`, busy `apply_liquid_metal` (`loading-shimmer`) | `Skeleton.tsx` (still pulses opacity; see the state contract) |
| Logo sheen | `assets/brand/fleetworks-material-logo.png`, `icons.rs::BRAND_MARK` | `packages/ui/src/FleetworksMark.tsx`, `assets/brand/fleetworks-mark.svg` |

`MetalFinish` variants: `GoldCta` (cool primary, name kept from the old theme), `Gunmetal` and `ObsidianPill` (neutral), `CrimsonDanger` (destructive entry point, such as an action that opens a confirmation). `Fractured` (Connect, Confirm, Use this CSV) and `FracturedDanger` (Delete from history) are the only textured finishes. When you add a button, pick a textured finish only if pressing it submits or confirms a change.

## 4. Fractured-obsidian texture

The texture is a `rune::Shape` painted by the button itself (`El::shape`), above the fill and below the label, so it adds no child, changes no layout and is invisible to the accessibility tree.

1. Fill the button with obsidian: `#0A0D12` (cool) or `#150A0E` (rose). Border 1 px in `STATUS_INFO_BORDER` (`#2B5E80`) or `STATUS_ERR_BORDER` (`#7D2B3F`). Hover steps to `#0F141C` or `#1D0E14`.
2. Build the shape in a 100 x 32 box with `view_box(100, 32)`, so it stretches to any button width. Inset every outer vertex at least 3 horizontally and 2 vertically so no facet pokes outside the 7 px rounded corner.
3. Corners `a (3,2)`, `b (40,2)`, `c (97,2)`, `d (97,30)`, `e (60,30)`, `f (3,30)`; interior points `p (34,14)`, `q (66,9)`, `r (80,20)`, `s (22,22)`.
4. Twelve triangles, alternating even and odd: `abp bqp bcq crq cdr rde qre pqe pef spf aps asf`. Even facets fill `TEXT_PRIMARY` at 4.5 % alpha; odd facets fill `CAST_SHADOW` at 24 % alpha.
5. Seven cracks, 0.6 pt, butt caps, in the border tint at 22 % alpha: `pq qr pe qe bp bq re`.
6. Ink is `TEXT_PRIMARY` (cool) or `STATUS_ERR` (rose). Disabled dims to 55 %.

Check: `material.rs` tests `only_the_two_submit_finishes_carry_texture`, `the_texture_has_facets_and_cracks` and `no_finish_paints_an_ivory_or_gold_fill`; the catalog scene (`catalog.rs`) draws every finish in default, busy, disabled and status states.

## 5. Loading shimmer

A skeleton is a flat `SURFACE_RAISED` block with a 1 px edge. When motion is allowed, one clipped band, 45 % wide, slides across it in 1.5 s with linear easing (`LoopSpec::slide`), tinted `TEXT_MUTED` at 10 % alpha (or the tone at 14 %). A busy button uses the same band at 16 %. Under Reduced Motion the block is static with identical size. A skeleton never covers readable text and is hidden from accessibility.

## 6. Contrast check

Run the shared contract and read the failures as numbers:

```bash
pnpm test packages/ui/__tests__/palette-contrast.test.ts packages/ui/__tests__/theme-tokens.test.tsx
```

It requires 4.5:1 for normal text on the canvas, panels, selected surface and translucent composites, 3:1 for essential borders and focus colors, 4.5:1 for every status and entity color on `#111720`, and checks the native constants against the web tokens. By hand: composite each foreground over its real background, convert to linear sRGB, and compute `(L1 + 0.05) / (L2 + 0.05)`. Decorative edges (`#262D38` and the 2 to 4 % cool-white card edges) never count as a boundary. Inspect keyboard focus (2 px `ACCENT_HIGHLIGHT` ring) on every state and confirm each status reads without its color.

## 7. Reproduce a screen

1. Start from `#060709`. Place content on `SURFACE`, raised rows on `SURFACE_RAISED`, hover and dialogs on `SURFACE_ELEVATED`.
2. Use `TEXT_PRIMARY` for values and titles, `TEXT_SECONDARY` for supporting copy, `TEXT_MUTED` for captions. Use weight and spacing to separate groups.
3. Draw controls flat: `Gunmetal` for utilities, `GoldCta` for the main non-submitting action, a textured finish only for submissions.
4. Add a status color only with a label or icon (`ui_components::status_pill`, `status_dot`).
5. For overviews, measure the container, choose columns from a content minimum (about 240 pt for summaries, 280 to 320 pt for options, gap 12 to 16 pt, at most four across), collapse to one column when needed, and keep the reading order stable when values change.
6. Capture the screen at `1280x800` and `640x480`, both scales, plus Reduced Motion and Reduced Transparency, and compare the exposure with the splash and the neighbouring service pages.

Standards: [text contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html), [non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html), [use of color](https://www.w3.org/WAI/WCAG22/Understanding/use-of-color.html).
