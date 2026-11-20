# Matte canvas theme

[Native control contract](../../wiki/code/native-design-system.md) · [Interaction and state](interaction-state.md) · [React guidance](../../wiki/code/react-ui.md)

@tag:look-and-feel @tag:react-ui

## Status and authority

This is the approved visual target for Fleetworks native and Intel Web, revised **2026-11-20 (America/New_York)**. It uses dark frosted glass, static liquid-metal rims, and warm accents in selected chrome. Persistent sheen and pointer lighting remain excluded. Record source integration, accessibility measurements, and production-view verification separately.

The audited source baseline is `nightly` at `00b700afd77ddac6cba2e2bd7ae91ab3802b7197` (tree `5051474185a8eb14b38e607480bb24c65352fff0`). Existing helper names below identify integration points, not already-implemented target behavior. The [native allowlist](../../wiki/code/native-design-system.md#complete-native-control-allowlist), connection contracts and web information architecture remain authoritative for controls and effects. This theme creates no new operation, authority, API call, chat surface or settings mode.

## Approved composition references

| Wide overview | Gateway setup |
| --- | --- |
| [![Workstation overview](mockups/01-workstation-status.png)](mockups/01-workstation-status.png) | [![Gateway setup](mockups/02-setup-gateway.png)](mockups/02-setup-gateway.png) |
| Activity and localized loading | Compact overview |
| [![Jobs and logs](mockups/03-jobs-logs.png)](mockups/03-jobs-logs.png) | [![Compact workstation](mockups/04-compact-workstation.png)](mockups/04-compact-workstation.png) |

These generated mockups establish composition and material direction. Numbers, hosts, identities, timestamps and platform labels are illustrative. They are not Rune/browser captures or evidence of backend state. The rules below resolve incidental image artifacts: the canvas is uniform, text is crisp, ordinary surfaces stay matte, and only genuinely pending regions receive loading motion. A displayed historical Ready result plus Stale describes an older observation, not current readiness.

## 1. Canvas, color and branding

Use an opaque **`#060709`** canvas for the window, web page, backend chooser and inset reading beds. Avoid stars, grain, vignettes or a wallpaper across working screens. The logo can contain its approved stars, molten core and reflective glass; its prepared asset edges must blend into this canvas or be transparent. Never stretch a poster to fill a logo slot.

Brand with the **symbol only**. Do not place a visible “Fleetworks” wordmark beside the mark or repeat it in page headings, cards or toolbars. Retain the product name in application metadata, OS identity, accessible naming where needed and documentation. Keep existing slot/API responsibilities, including `FleetworksMark` and native `BRAND_MARK`; replacing their artwork is implementation work. At small sizes use the documented silhouette fallback, preserving one crescent and one molten sphere. Large backend chooser art may retain material detail. Do not repeat reflective logo treatments on controls.

### Exact target tokens

The role names are the design contract. Native/shared-web mappings reuse existing exports where shown; extra roles require explicit new tokens. Values are sRGB hex. Keep text fully opaque rather than fading a whole content subtree.

| Role | Value | Existing integration point / use |
| --- | --- | --- |
| Canvas / inset | `#060709` | `BACKGROUND`, `SURFACE_INSET`; `colors.bg`, `colors.inset` |
| Group body | `#0B0F14` | `SURFACE`; `colors.panel`; masked group fill |
| Raised control | `#121820` | `SURFACE_RAISED`; `colors.raised` |
| Elevated surface | `#19212B` | `SURFACE_ELEVATED`; dialogs/menus, still opaque |
| Primary text | `#D5DCE5` | `TEXT_PRIMARY`; `colors.text` |
| Supporting text | `#A8B6C5` | `TEXT_SECONDARY`; `colors.dim` |
| Muted text | `#8192A4` | `TEXT_MUTED`; `colors.muted` |
| Disabled text | `#7F8D9D` | New role; retain a readable reason |
| Functional boundary | `#647587` | `BORDER_CONTROL`; `colors.border` |
| Decorative divider | `#2B343F` | `BORDER_SUBTLE`; `colors.borderSubtle`; never sole control boundary |
| Interactive accent | `#24DCE3` | `ACCENT`; `colors.accent`; selected icon/label and action ink |
| Accent hover / pressed | `#68E7EB` / `#1BB8C2` | Existing accent-state exports |
| Focus | `#72E9FF` | `ACCENT_HIGHLIGHT`; dedicated web focus role |
| Selection fill | `#10272D` | `SELECTION_SURFACE`; `colors.selectionSurface` |

Use `#E5B869` molten-gold ink and `#241D12` warm tint on active navigation, unread counts, and selected links. Use cyan for general interactive affordances and keep the existing warning and chart colors recognizable.

### Flat semantic and chart palette

| Meaning | Color | Application |
| --- | --- | --- |
| Success / ready | `#23DFA0` | Check plus state text |
| Warning / stale | `#F1C84B` | Warning/clock plus explicit qualification |
| Failure / destructive | `#F280A0` | Error mark, diagnostic or destructive intent |
| Information | `#53BAF5` | Informational state, not success |
| Pending / selected | `#24DCE3` | Truthful pending label or selected affordance |
| Unavailable / neutral | `#95A4B5` | Unavailable label, never invented zero |
| CPU / memory / disk / network | `#24DCE3` / `#BA77F2` / `#35AFFF` / `#F28C65` | Distinct flat metric series |
| Additional categorical series | `#63D2BD`, `#B599EA`, `#F28BA8`, `#A0BBC9` | Stable category mapping with labels/legend |

Retain existing evidence, entity, severity and metric meanings when mapping `StatusTone`, `intelStatus`, metric/kind colors and topology tokens. Do not assign colors by row position on each refresh. Charts use solid strokes/fills, with no glow, sheen, textured arcs or decorative gradients. Threshold warnings remain explicit through their existing state cues; a measured category color alone is not a health claim.

## 2. Deep-space surfaces, frosted glass and liquid-metal rims

Group related content into calm, spacious tonal regions over the `#060709` deep-space canvas:

- **Primary content groups & settings containers:** Use quiet `#0B0F14` surfaces with either a clean `1px` hairline border (`#2B343F` with subtle horizontal row dividers inside grouped settings blocks, matching the reference Settings composition) or the soft perimeter fade (`GroupEdge`: 20px standard / 16px compact radius). Keep open grids uncluttered at rest, revealing a soft lifted surface (`#121820`–`#19212B`) on hover or selection. Never stack heavy boxed borders around every individual tile.
- **Floating glass overlays (Inbox/Notifications popover, Command palette, Context menus, Modals):** Render as **dark frosted glass** using same-window backdrop blur (`10–16px`), translucent deep-slate fill (`#121820` at `80–90%` alpha), and a **crisp 1px liquid-metal specular top/outer rim** (`#D5DCE5` at `12–22%` alpha fading into `#2B343F` along the bottom/sides). For grouped notification/inbox stacks anchored under the top-right bell badge, use floating rounded glass cards with subtle peeking stacked-card lips underneath grouped items instead of an opaque full-height side slab. Under reduced transparency (`ViewPreferences::matte()` / high contrast), fall back to opaque `#121820`/`#19212B` with the exact same bounds and crisp `#647587` borders.
- **Desktop shell chrome:** Follow the reference shell rhythm where applicable — slim dark left navigation rail with symbol-only brand mark at top, subtle rounded active navigation pill (warm molten-gold/amber tint or clean metallic highlight), centered pill-shaped command trigger (`Run a command`) in the top bar, and a compact top-right telemetry/status & notification bell strip.

Retain `GroupEdge` (`material.rs`) and `groupEdge` (`packages/ui/src/index.tsx`) where diffuse perimeter blending is used: standard spread 8px / blur 7.5px (`20px` radius, `24px` inset) and compact spread 6px / blur 5.6px (`16px` radius, `20px` inset). Reading/log/table interiors remain solid canvas for legibility.

### Functional exceptions

Fields, controls and independent selected items retain a **1px `#647587`** boundary (or highlighted active border on selected option cards) where needed for identification. Keyboard focus uses a **2px `#72E9FF`** outline, a **2px canvas-colored gap**, and an unclipped outer allowance. These boundaries stay crisp and independent of decorative highlights or glass blur. Never weaken them to make the background softer.

## 3. Crescent moon textures and liquid-metal action highlights

Carry the **fractured-obsidian crescent texture** (`assets/brand/rock/fleetworks-rock-tile.*`) and **restrained liquid-metal edge highlights** through the UI in harmony with the logo:

- **Committing semantic actions (Submit, Confirm, Create, Delete, Connect):** Carry the coarse crescent-rock texture with semantic tint and a subtle 1px liquid-metal upper rim highlight (`#D5DCE5` at low alpha) so primary commitments feel tangible and distinct from utility controls.
- **Brand/hero accents:** Featured brand headers, backend-choice tiles, and selected high-emphasis cards may pair subtle low-opacity crescent-rock texture or a brushed liquid-metal top bevel with crisp opaque typography above it, provided text contrast stays ≥ 4.5:1.
- **Routine utility controls:** Cancel, filters, tabs, search inputs, inline dropdowns, segmented pills, read retries, and compact list rows stay clean smooth slate or frosted glass with crisp hairline/metallic borders rather than heavy texture noise.

### Deterministic asset and tint recipe

Derive one grayscale texture from the **coarse rock interior of the approved crescent**. Preserve angular fragments and dark crevices while keeping luminance controlled so foreground labels always exceed contrast requirements. Record source hash, crop, normalization and export settings in the asset manifest; do not synthesize new noise at runtime.

- Prepare one **512×512** source, displayed on a **256×256 logical-pixel** tile. Keep dominant rock fragments approximately **12–28 logical pixels** wide. Use the same scale on every textured surface; repeat/crop rather than stretch to each control's aspect ratio.
- Anchor texture coordinates to the control's top-left content-independent bounds. Preserve that origin while its label, busy state or width changes. No drifting, parallax or random phase on rerender.
- Normalize the grayscale sample to `T` in `[-1,1]`, with mean zero. Modulate each base sRGB channel by `round(50 * opacity * T)`, clamped to the valid channel range. Rest opacity **0.12–0.16** gives tactile obsidian relief without hurting legibility.
- Use **10px corner radius**, **44px preferred control height**, at least **16px horizontal padding**, and an unchanged hit rectangle through all states. Text and icon sit above the texture at full opacity.

| Intent | Base fill | Ink/context tint |
| --- | --- | --- |
| Submit / create, including Connect | `#10262C` (or molten highlight `#241D12` where primary brand CTA applies) | `#24DCE3` / `#E5B869` |
| Confirm | `#10281F` | `#23DFA0` |
| Delete / destructive confirmation | `#2B1823` | `#F280A0` |

Context tint affects only the eligible action and its existing semantic indication. Do not wash a whole page red/green/cyan/amber. Preserve readable labels and the functional border; tint is supplementary to the action name and confirmation.

| State | Deterministic treatment |
| --- | --- |
| Rest | Base above, texture opacity `0.12`, crisp hairline / subtle liquid-metal top edge, no continuous wake loop |
| Hover | Lift fill +2–4 levels, texture opacity `0.14–0.16`, brighten top rim slightly; 120ms transition only |
| Pressed | Darken fill -2 levels, texture opacity `0.10`; no scale/bounce or moving texture |
| Focus | Same material plus independent 2px `#72E9FF` focus outline |
| Disabled | Raised-control base, disabled ink, texture opacity `0.04`; retain reason and semantic disabled state |
| Pending | Keep the same material/label geometry; only the allowed loading layer below may move |

### The committed crescent tile (implemented)

The tile is derived from the approved crescent source by [`derive_rock_tile.py`](../../../assets/brand/source/derive_rock_tile.py) and committed under [`assets/brand/rock/`](../../../assets/brand/rock/): `fleetworks-rock-tile.png` (web), `fleetworks-rock-tile.rgba` (native, the same pixels) and `fleetworks-rock-tile.manifest.json` (source hash, crop, normalization, export settings and output hashes). Native loads the raw RGBA into a Rune `Pixels` once per UI thread; web embeds the PNG as a data URI (`packages/ui/src/rockTileData.ts`, generated by the same script). Nothing is generated at runtime.

How it is derived, so another maintainer can repeat it:

1. **Source and crop.** The source is `assets/brand/source/fleetworks-liquid-metal-glass.png` (SHA-256 `1ae55378…70112`, checked by the script). The crop box is **x 655, y 715, 256 × 96 source pixels**, inside the crescent's dark rock, clear of the polished rim, the molten sphere and the large lit facets near its edges.
2. **Exclusions.** Luminance uses `0.2126 R + 0.7152 G + 0.0722 B` on the sRGB bytes. Glowing inclusions (the warm star glints, luminance above 150) are replaced by the median of the dark rock around them within 10 px, and the rest is capped at the crop's 97th percentile so highlights stay out.
3. **Tile.** The crop is stacked with its vertical mirror to fill a 256 × 256 block, then mirrored on both axes to **512 × 512**. Mirror quilting makes the tile repeat without a seam. It is shown at a fixed **256 × 256 logical pixels** (2 source pixels per logical pixel), anchored at the control's top-left, and repeated on both axes, never stretched. Known limit: the mirror symmetry is visible in the tile itself, but at rest opacity 0.12–0.16 it reads as rock; replace the crop box with a larger approved rock plate when one exists.
4. **Normalization.** `T = (Y − mean) / p99(|Y − mean|)`, clipped to [−1, 1], re-centered, then quantized to steps of 1/16. The manifest records mean luma, scale and a residual mean of T near 0.0001.
5. **Export.** One RGBA overlay at opacity 1: `T > 0` is `#D5DCE5` at alpha `0.22 T`; `T < 0` is `#020304` at alpha `min(1, 1.25 |T|)`. At opacity `o` a button base channel changes by about `round(50 · o · T)` levels. State opacities: rest 0.12, hover 0.14, pressed 0.10, disabled 0.04 (native draws pressed as the rest texture over a 2-level darker fill).

Reproduce or verify it (Python 3, standard library only):

```sh
python3 assets/brand/source/derive_rock_tile.py          # rewrite the tile, manifest and web module
python3 assets/brand/source/derive_rock_tile.py --check  # fail if the committed .rgba differs
```

Native lays one image per 256 px of the control's measured width (`texture_tiles`), each with its opacity bound to the control's hover group, inside one hidden, rounded-clipped layer over the button fill. Web draws the same PNG as the background of an overlay layer inside the button, with the state opacity on the layer. Native hover raises the texture to 0.14 and adds 2 levels to the fill; pressed subtracts 2 levels from the fill, computed on the encoded sRGB bytes (`#10262C` becomes `#0E242A`) and converted back to Rune's linear channels, and keeps the rest texture, because Rune has no pressed style for descendants. A control taller than one 256 px tile, or wider than 64 tiles, is flat beyond it.

Normal text must reach **4.5:1**, large text **3:1**, and essential boundaries/focus **3:1** against adjacent colors after compositing. Reference calculations on the opaque tokens give primary/supporting/muted text **11.75/7.85/5.09:1** against the elevated surface, functional boundary **3.43:1**, and destructive ink **6.66:1** against its untextured base. These calculations are design checks, not rendered accessibility approval; verify the brightest texture samples, glass/rim highlights, antialiasing and every state in production views.

## 4. Motion: loading-only translated sheen and static liquid-metal edges

Liquid-metal rim highlights and glass bevels are **static at rest** (no continuous resting animation wake loop, no blinking glare, and no fixed-pixel pointer-lighting loop). The Quiet Orbit emblem remains the sole continuous ambient motion subject, governed by the [Quiet Orbit motion contract](motion.md).

A genuinely pending, unresolved region may use a restrained translated loading band over a **steady base**:

- Skeleton base **`#27313B`**, constant opacity 1. Band color **`#A9C8D9`**, peak alpha **0.06**, transparent at both edges; band width **20% of the local region width**, softly feathered across its full width.
- Use a **1.6s total cycle**: translate linearly across the clipped region for **1.2s**, then remain outside the bounds for **0.4s** before repeating. The base and text never pulse, fade or blink. Use a shared phase per region, not an independently flashing loop on every row.
- Clip to placeholder geometry or the pending button's background; never sweep across labels, resolved values or retained rows. Do not add a fake percentage. Stop on resolution, failure, unmount, hidden/unfocused view or reduced motion.
- Reserve geometry and keep one local status owner. A read refresh retaining valid data does not become a full-screen loading effect. See the [state contract](interaction-state.md) for initial loads, generation-stage exceptions, retained refresh and continuation.

The source currently has native `LoopSpec::slide` effects and a web `Skeleton` opacity loop. Reusing the native mechanism does not preserve its old amplitude; replace the web pulsing behavior rather than masking it with CSS alone. Use no fixed-pixel pointer-normalization reference such as `1180×780`.

## 5. Grids, typography and responsive composition

Use grids for dense overview, option and status groups; preserve tables when aligned columns support comparison. Retain native destinations, web navigation, Bulletin's record list, Brief reading order and topology's accessible list. A grid changes arrangement, not authority or interaction meaning.

Measure **available content width**, after sidebar/rail/insets. With gap `g=16px`, choose `n = clamp(floor((width+g)/(minimum+g)), 1, maximum)` and card width `(width-(n-1)*g)/n`. Use these initial targets:

| Group | Minimum cell width | Maximum columns |
| --- | --- | --- |
| Resource metrics | 240px | 3 |
| Native Client / Gateway Backend | 280px | 2 |
| Readiness / compact options | 200px | 3 |

Allow taller cells for wrapping; never shrink labels to preserve a column count. Preserve source/keyboard order as columns collapse. Below the minimum required width, use one column; hide only the existing optional context rail, not required actions. Native retains **640×480** support; web checks remain **1440/768/390/320px**. At large text scaling, reduce columns before clipping. Do not force every short status into its own bordered card.

Use the existing native Kit and web font families. Target native logical sizes: page heading **26/32px**, section **18/24px**, body/control **15/22px**, supporting **13/18px**, metric value **28/32px**; weights 600 for headings/metrics and 400–500 for prose. Use existing web density/type mappings rather than adding global zoom or replacing its navigation/prose font roles. Machine IDs/logs use the current mono family; changing metrics use tabular numerals and reserved width. No text glow, blur, letter animation or rolling digits on live refresh. Keep labels short, diagnostics local and necessary details available without permanent explanatory paragraphs.

## 6. Integration, preferences and evidence

| Verified source authority | Target integration |
| --- | --- |
| [`theme.rs`](../../../apps/fleetworks-desktop/src/theme.rs), [`material.rs`](../../../apps/fleetworks-desktop/src/material.rs) | Map deep-space tokens, static liquid-metal rim highlights, frosted `GlassSurface` overlays (`backdrop_blur` + rim border with matte fallback), `GroupEdge` / hairline grouped cards, crescent `rock_texture`, and `skeleton_block`. |
| [`ui_components.rs`](../../../apps/fleetworks-desktop/src/ui_components.rs), [`inbox.rs`](../../../apps/fleetworks-desktop/src/inbox.rs), [`app.rs`](../../../apps/fleetworks-desktop/src/app.rs) | Keep `ActionButton`, status/metric/skeleton primitives, floating glass Inbox popover & context menus, top command/telemetry chrome, and clean hairline-grouped Settings/content rows. |
| [`accessibility.rs`](../../../apps/fleetworks-desktop/src/accessibility.rs) | Reuse `Accessibility::for_view` and `ViewPreferences::matte()`; retain opaque fallbacks for reduced transparency and identical hit bounds. |
| [`packages/ui/src/index.tsx`](../../../packages/ui/src/index.tsx) | Extend `colors`, `Frame`, `Panel`, `Button`, `glass` (frosted backdrop-filter + hairline liquid-metal rim with reduced-transparency fallback), `RockTexture`, and shared recipes. |
| [`intel-theme.ts`](../../../packages/intel/src/shell/intel-theme.ts), [`style.css`](../../../apps/web-desktop/app/style.css), [`Skeleton.tsx`](../../../packages/ui/src/Skeleton.tsx) | Coordinate deep-space semantic tokens, molten/cyan highlights, focus ring, frosted overlay glass and steady-base loading without changing state ownership. |

Reduced motion uses static placeholders and immediate state changes. Reduced transparency (`ViewPreferences::matte()`, CSS `prefers-reduced-transparency: reduce`) replaces translucent backdrop blur with opaque `#121820`/`#19212B` fills while preserving bounds and crisp borders. High-contrast needs take precedence: use solid group boundaries if needed, remove rock modulation, retain text and focus. Do not add a preference toggle just for this theme.

Cache the one rock texture and reusable mask resources. A 512² RGBA upload is about **1 MiB**; share it instead of one copy per tint/button. Generate neither textures nor full-size masks during each render. Resting screens must have no decorative animation wake loop. Animate only visible unresolved regions; stop resources with their owner. Profile the actual native/browser implementation before claiming performance.

Acceptance requires inspected production views: canvas-matched symbol-only crescent + molten-core logo, deep-space hierarchy, frosted-glass floating overlays with crisp liquid-metal rim highlights (and opaque reduced-transparency fallbacks), crescent-rock texture on eligible committing/brand surfaces, clean reference desktop layout rhythm (rail, top command/telemetry bar, hairline-grouped settings, floating inbox stack), clear semantic colors, retained-data stability and keyboard focus in all preferences. Inspect 1×/2×, compact/wide, hover/press/focus/disabled/pending/error and long text. Record implementation, static checks, runtime captures and installed-app/browser review separately. The mockups and this documentation do not establish any of those runtime passes.
