# FLEETWORKS_BRAND_SPLASH_DEEP_SPACE_AND_LIVE_STATE_IMPLEMENTATION_SPEC

> **Status in this repository (read first).** This is the owner's handoff, kept as the design intent. The repository applies it with two differences, both decided by the owner's [PR addendum](../../../specs/fleetworks-native-web-ui-overhaul.md#addendum-quiet-orbit-matte-theme-and-stable-live-states):
>
> 1. **Theme is matte.** Where this file or the [theme contract](fleetworks-deep-space-theme.md) once asked for liquid-metal controls, glass bodies or pointer lighting, the addendum wins: sheen only on the main logo, a cool shimmer only while loading, fractured-obsidian texture only on Submit, Confirm, Create, Delete and Connect.
> 2. **The splash is vector.** `apps/fleetworks-desktop/src/orbit.rs` draws the scene from geometry, so no raster layer pack ships (0 bytes of splash runtime assets; budgets of 6 MiB shipped and 64 MiB decoded are met trivially). The PNG and GIF stay in `assets/brand/` as canonical references. The "bounded RGBA layers" in the edit map below are the upgrade path if a richer render is wanted; the layer sources are not in the repository.
>
> Where the text below names `splash.rs`, read `orbit.rs` (geometry, scene) and `startup.rs` (timeline, view). The timeline is the one below: scene until 2.65 s, crossfade to the connection choice until 3.00 s. The choice itself is today's "Choose your connection" screen, not a separate setup page. Step-by-step reproduction is in [Reproduce](#reproduce).


**Source baseline:** `xxxcess/argos-osint` · `nightly` · `00b700afd77ddac6cba2e2bd7ae91ab3802b7197` · rechecked **2026-11-16 America/New_York**. Audited sources are verified against the pinned tree in `repository-audit.json`; a divergent local React guide was preserved and the pinned remote version inspected. Snapshot has no Git metadata; dirty status N/A. This revision updates implementation requirements and the asset handoff; product changes/builds/tests remain execution work.

## Outcome and audited conclusions

Adopt the approved liquid-metal/glass logo and **Quiet Orbit** throughout repository branding. Carry a consistent deep-space/liquid-glass finish from the three-second splash through gateway setup into native services. Fix flickering loading/state text, shorten section copy and use responsive grids to make dense information/options/status easier to scan. **`fleetworks-deep-space-theme.md` and `fleetworks-realtime-state-contract.md` are mandatory companion contracts.**

- Audited startup renders the shell directly: `launch::run → app::spec → app::view`. Add a finite setup/poll/view phase.
- Reuse `views::page` routing `State::setup_gateway = true` to `setup_gateway`. Avoid `State::navigate`, which resets that flag and can interrupt pairing.
- Pinned Rune supports asynchronous images/transforms. GIF frame/mip decoding can exceed its animation budget and trigger still fallback; use bounded layers.
- Native header/launcher diamond/F icons differ; replace their identity with the approved emblem family, using readable compact derivatives.
- App/web canvas is `#060709`, while the review GIF has black edges; compose transparent scene layers over one persistent full-window background.
- Native material helpers already exist, but Linux forces matte fallback and pointer lighting uses fixed 1180×780 dimensions. Separate material capability/preferences and use measured bounds.
- Shared web `glass` currently aliases matte paint; maintained guidance/tests prohibit gloss/blur. This user direction supersedes those paint limits. Preserve shared component, state, focus and accessibility contracts.
- Resolve loading, refresh, transport and domain state separately. Source findings and required regression cases for the reported text flicker are recorded in the state contract; do not claim visual reproduction from source inspection.

**Material reference:** `fleetworks-liquid-metal-glass-baseline.png`, SHA-256 `1ae553782431da7649a1d88accdacc9076c3a039f1c2b5c0018f5998d7d70112`. Preserve this approved artwork and the selected GIF byte-for-byte as canonical sources; derive runtime assets faithfully.

Four studies are superseded alternatives. See `fleetworks-motion-reference.md` for selected Quiet Orbit. Its review loop: **8 seconds, 160 frames, 20 fps**, seamless without opacity fades; native timing remains separate.

## Repository asset adoption

`repository-adoption.json` maps supplied files and measured hashes to these destinations. Proposed additions are explicitly new.

| Destination | Role |
|---|---|
| `assets/brand/fleetworks-material-logo.png` **(new)** | Byte-identical approved PNG; canonical rich logo and README hero. |
| `assets/brand/fleetworks-quiet-orbit.gif` **(new)** | Byte-identical selected GIF, 800×450; canonical motion reference. |
| `assets/brand/derived/` **(new)** | Transparent rich mark and square safe-inset platform master, with provenance. |
| `assets/brand/splash/` **(new)** | Bounded runtime RGBA layers/core sequence, transparent complete still and normalized layout/orbit data. |
| `assets/brand/{brand-manifest.json,README.md}` **(new)** | Source/derivative roles, hashes, dimensions, alpha/color space, anchors, timing, reproducible exports, clearance and measured runtime footprint. |

Retain `assets/brand/fleetworks-{logo,mark}.svg` as the explicit small-size vector family with one crescent, one disc and five triangles. At 18-pixel header sizes use that readable geometry; do not shrink a widescreen poster into the slot. Use rich artwork where texture is legible. Retain the old JPG as historical source while switching active consumers. Native packaging supports SVG/PNG/ICNS but excludes GIF: keep the reference GIF under canonical repository assets and ship the bounded native rendering assets.

## Edit map

| File / symbol | Required change |
|---|---|
| `apps/fleetworks-desktop/src/app.rs` — `State`, `spec`, `poll`, `view`, `on_key` | Splash state, cache, clock, input ownership and completion. |
| `apps/fleetworks-desktop/src/launch.rs::run`; `apps/fleetworks-desktop/src/lib.rs` | Arm GUI launch; register module; preserve initialization. |
| `apps/fleetworks-desktop/src/splash.rs` **(new)** | Layers, orbital depth/clearance, timeline, completion and tests. |
| `apps/fleetworks-desktop/src/{theme.rs,material.rs,accessibility.rs,ui_components.rs}` | Shared space/glass/metal recipes, production preferences, semantic colors, skeletons and stable state indicators. |
| `apps/fleetworks-desktop/src/icons.rs::BRAND_MARK`; `resources/fleetworks.svg`; `resources/Info.plist` under that app | Replace compact diamond/Linux F; add the declared `resources/fleetworks.icns` derivative. |
| `packages/ui/src/{FleetworksMark.tsx,index.tsx}`; `packages/intel/src/shell/{IntelShell.tsx,intel-theme.ts}` | Shared web identity, material/color tokens and real component states. |
| `apps/web-desktop/app/{style.css,layout.tsx,icon.svg,apple-icon.svg}`; `public/fleetworks-mark.svg` under that app | Shared canvas, skeleton/focus/preferences, synchronized favicon/icon metadata and any exact new `public/brand/` derivatives. |
| `apps/web-desktop/connect-session.mjs::isPublicWebPath` | Permit only exact new pre-connection asset URLs when required, respecting base paths and authentication fences. |
| `scripts/fleetworks_candidate.py::{_resource_files,package_native}`; `scripts/test_fleetworks_candidate.py` | Reuse supported resource packaging and validate declared platform icons. |
| `apps/fleetworks-desktop/examples/native_fixture.rs`; `apps/fleetworks-desktop/examples/native/splash_scenes.rs` **(new)** | Production-state/view scenes for complete materials, orbital depth, core, handoff and fallback. |
| Native `src/appearance_tests.rs` and `examples/native/appearance_scenes.rs`; `packages/ui/__tests__/{palette-contrast.test.ts,theme-tokens.test.tsx}` | Actual view/material parity, semantic contrast, canonical geometry, changed glass/opaque recipes and state transitions. |
| `README.md`; native `README.md`; `docs/wiki/code/{native-design-system.md,react-ui.md}` | Current branding, corrected source palette, splash/theme/state guidance and durable visual evidence. Reconcile related human-maintained theme wording in root/web `AGENTS.md`; preserve workflow rules and generated blocks. |
| `.github/workflows/{fleetworks-native-linux.yml,fleetworks-native.yml,intel-web.yml}` | Code Architect updates affected evidence scenes if needed; retain supported target matrices. |

## Implementation

1. **Prepare assets and consumers.** Commit the canonical PNG/GIF at the mapped paths; derive scene/platform assets and update active README, native header/launcher and web header/favicon consumers. Preserve source shape proportions/materials and keep complete provenance. Bundle runtime bytes with `include_bytes!` for offline/relocated startup. Remove outline-first, flat-to-glass, loading-reveal and assembly stages. Fine white glass reflections remain.

2. **Reuse Rune.** Create one `ImageCache::new(waker)` in `AppSpec::setup`; load once and inspect readiness/errors asynchronously. Draw through `Image::pixels`, `El::pixels_fit(..., SurfaceFit::Fit)` and existing absolute layers/transforms. Use a localized pre-rendered sequence for slow shell rotation and independent molten flow inside a stable spherical boundary. Decode/synthesize outside `view`; reuse handles and the pinned dependency set. Add no embedded webview, video player, FFmpeg runtime or separate 3D engine. Measure budgets of **≤6 MiB shipped splash/brand runtime assets and ≤64 MiB decoded splash assets including mips**; repository review media are not shipped runtime assets.

3. **Implement the revised scene.**
   - **Opening/background:** fully formed materials and ongoing animation in the first visible logo frame; nearly black space, substantial negative space, sparse dim stars and faint widely separated inclusions.
   - **Crescent:** fixed upward-facing glass-edged enclosure containing coarse obsidian rock: irregular angular fragments, rough relief and dark crevices beneath a polished boundary. Keep this interior legible at normal size, with restrained edge highlights. Tiny diamond/star inclusions may glow faintly within the glass, widely spaced so the coarse rock remains dominant.
   - **Sphere:** dark ivory/smoky glass enclosing emissive, viscous molten liquid. Show internal flowing lobes, convection and volumetric depth through shell reflection/refraction, with restrained glow. Painted marbling or whole-disc spin is insufficient.
   - **Satellites:** exactly five persistent thick obsidian-glass triangular prisms with dark cores, dimensional facets and restrained reflections. Use the elevated, inclined full orbit verified in `orbit-clearance.json`, visually encircling the sphere outside its volume through upper/lower and front/back passes. Depth-sort against the sphere: rear satellites are naturally occluded, foreground satellites remain in front. Add modest perspective scaling and facet attitude changes while preserving shape and spacing.
   - **Orbit:** at reference width 800 use **130-pixel radius, 66° inclination, 38-pixel upward projection offset, 0.70 prism scale and 72° phase spacing**. Preserve the continuous full track and slow eight-second pace; do not accelerate to finish a revolution during startup.
   - **Hard crescent clearance:** the entire transformed satellite silhouettes—including perspective, rotation, bevels and an antialias margin—must remain visibly separated from the crescent at every phase and supported size, in scene space and screen projection. Adjust trajectory geometry; never conceal collisions by hiding/clipping triangles or drawing the crescent over them. Planet occlusion remains allowed. Record final geometry and minimum clearance.
   - **Layout:** let `B` be the union of all objects over the complete orbit, including bevel/glow/alpha and clearance margin. For usable client bounds `W×H` after insets, set `P = clamp(0.04×min(W,H),16,32)` logical pixels and target height `min(0.66×(H−2P),440)`. Uniform scale is `min((W−2P)/B.width,targetHeight/B.height)`; center `B`. Transform all layers together. Contain the whole orbit at 640×480; test fractional DPI and 1×/2×. Resize cannot restart phase or timing. No words, composer, percentage or extra controls in the splash.
   - **Canvas continuity:** keep native `theme::BACKGROUND` / web `colors.bg` **`#060709`** as one shared window background. Derive RGBA coverage from silhouettes; black obsidian interiors stay opaque, so global black removal is invalid. Record alpha/color space and use it consistently through scaling/mips. Exterior stars/atmosphere belong to the persistent full-window canvas; inclusions belong to objects. Crossfade foregrounds while retaining the same background under splash, gateway and shell. Complete fallback stills use the same coverage. No image-sized vignette, black rectangle, pale fringe or clipped glow. Normalize pointer lighting against measured shell bounds, retaining `Live`/`tx.live_only()`.

4. **Run once per cold GUI process launch.** Anchor timing to the first `app::poll` after arming, independently of image readiness. Preserve other poll work and concurrent settings initialization/retained-credential reconnect. Keep help/version, credential provisioning and ordinary non-splash fixtures on their existing direct paths.

   | Time | Behavior |
   |---|---|
   | 0.00–2.65 s | Complete scene: satellite orbits, slow sphere rotation, internal molten flow. |
   | 2.65–3.00 s | Crossfade into existing gateway setup; retain splash input ownership. |

   At 2.65 seconds, set `setup_gateway = true` beneath the input-blocking splash; retain `Page::Workstation` and saved/pending connections. At 3.00, complete once, stop decorative wakes, release assets and transfer input/focus. Close/quit also releases splash handles and cancels decorative wakes. Create no pairing/backend intent. Network/decode delays cannot extend the deadline; late wakes enter setup immediately. Navigation, reconnect, resize and resume never replay startup.

5. **Input/accessibility/fallback.** Block underlying clicks/shortcuts; retain native close/quit. Focus the first enabled setup control after rendering. Reuse `Accessibility::for_view`: reduced motion shows the complete still with the same handoff, disabling orbit, rotation, molten flow, parallax, twinkling and value tweening. Reduced transparency uses opaque control fills with restrained baked reflections; logo coverage still composites onto the shared canvas. Unavailable motion layers use the revised still; failed stills proceed to setup. Hide decoration from accessibility; provide one concise launch label. Existing fixtures bypass startup unless opted in; preserve `fixture-marker`.

6. **Apply the deep-space theme and grids.** Implement the mandatory companion using shared authorities. Give the canvas quiet depth, panels dark glass bodies, and interactive surfaces dimensional liquid-metal highlights. Use semantic cyan/blue/green/amber/rose/violet alongside brand ivory/gold. Carry the same material quality, exposure and reflection temperature through splash → gateway → native services, including skeleton/live/error states. Use measured responsive grids for dense summaries/options/status, organized overview → exceptions → detail, retaining tables for record comparisons and stable card positions during updates. Separate safe native shading from optional translucency/refraction and expensive motion; Linux production must retain dimensional material styling instead of unconditionally suppressing every rich effect. Preserve geometry/input parity with opaque/reduced variants and update old matte-only docs/tests deliberately. Creative refinement is allowed within identity, readability, motion and performance limits.

7. **Resolve realtime display state and shorten copy.** Implement the mandatory state contract through the affected native/shared web boundaries. Stable headings and values must not alternate with polling/loading sentences. Use layout-matched shimmer skeletons for unresolved content; retain settled content during same-scope refresh with a small reserved indicator. Keep transport health, request activity and domain outcomes separate. Preserve authoritative stage placeholders, request/scope ordering, unknown/zero/empty distinctions and input/scroll stability. Prefer concise labels, local indicators and expandable detail to repeated explanatory text. Do not add a second transport or drive network refresh from an animation clock.

## Acceptance and validation

- Inspect complete first-frame materials, readable coarse rock and internal convection independent of shell rotation; confirm five persistent satellites and correct orbital occlusion/perspective.
- Sweep the full cycle densely, beyond exported-frame cadence, at supported sizes/scales. Include transformed silhouettes, bevel/antialias and interpolation margins. Record minimum crescent clearance and screenshots at its worst phases; prove all five objects persist and no concealment masks intersections.
- Outside object/glow coverage, composites equal the shared canvas within one 8-bit channel level. Inspect production and diagnostic backgrounds at fractional scales for rectangular seams/halos and splash-to-gateway exposure jumps. Resize through native breakpoints; inspect real Linux production preferences as well as fixture overrides.
- Review the complete splash → gateway → service loading → settled/live service sequence at the same size/exposure. Require consistent canvas, palette, glass/metal finish and crispness throughout; reject a polished splash followed by visibly mismatched service surfaces. Check grid high-level readability, keyboard/reading order and skeleton-to-content geometry with rails/panels open and closed.
- Inspect material states and semantic color across buttons, toggles, labels, metrics, gauges, charts, icons, dialogs and dense logs. Check contrast after compositing, reduced preferences and keyboard parity. Verify the state-contract sequences with controlled async ordering: no text/skeleton flicker during background refresh, no stale-response regression, immediate truthful completion/failure, stable layout and concise copy.
- Controlled clocks at start, before/at 2.65 and 3.00 seconds and a late wake prove one completion, correct crossfade target, retained reconnect, no backend intent or stale wake/replay.
- Verify reduced motion, unavailable/failed assets, close during orbit, focus and blocked underlying input. Capture actual Runtime views at **640×480 and 1600×900, 1×/2×**, including initial materials, depth and setup after 3.1 seconds. Explicit splash fixtures may `Image::wait` before advancing seeded virtual time; interactive startup remains asynchronous.
- Run `cargo fmt --all -- --check`, then focused new splash/state cases and `python3 scripts/agent_cargo.py test --locked -p fleetworks-desktop -- --test-threads=1` within authorized scope.
- Use `python3 scripts/fleetworks_verify.py native-ui --agent --scene <registered-splash-scene> --at 1.8 --size 640x480 --size 1600x900 --scale 1 --output .agent-scratch/native-ui/splash-material`; register scenes first and repeat affected times/scales. Reuse the shared cache/visual-review workflow; iterate Linux first.
- For shared web changes, run affected real-component/state tests, including `pnpm test packages/ui/__tests__/palette-contrast.test.ts packages/ui/__tests__/theme-tokens.test.tsx`, plus `pnpm typecheck`, `pnpm lint` and affected repository Playwright under the authorized scope. Inspect 1440/768/390/320-pixel widths. Reuse package/icon tests and exact-revision visual jobs.
- **Evidence:** source audit and reference preview QA are complete: 1,440 phases, zero sampled painted-crescent overlap and approximately eight-pixel clearance with stated reserves. Product flicker reproduction, native compositing/timing, builds/tests, captures, resource measurements and installed-app verification remain pending. Previews establish visual intent only.

Execution: verify baseline/symbols and relevant HEAD changes; implement this handoff and both companion contracts, run required authorized checks, fix/retest demonstrated failures, and report changed files, actual outcomes, inspected evidence, measured footprint/performance and blockers.

### Pinned implementation references

- [Audited repository baseline](https://github.com/xxxcess/argos-osint/tree/00b700afd77ddac6cba2e2bd7ae91ab3802b7197)
- [Native lifecycle and state](https://github.com/xxxcess/argos-osint/blob/00b700afd77ddac6cba2e2bd7ae91ab3802b7197/apps/fleetworks-desktop/src/app.rs)
- [Rune image cache](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune/src/image.rs), [drawing/transforms](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune/src/el/visuals.rs), [setup/poll](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune/src/tooling.rs), [animated-image budget](https://github.com/anomalyco/rune/blob/205b8d4f7e60fa6248b83bfbbb4c6a06096c59ff/crates/rune/src/image/animated.rs).

## Reproduce

Everything needed to rebuild the splash, check it and re-export the review media is here. Nothing depends on the original zip.

### 1. What you are rebuilding

| Item | Value |
|---|---|
| Canvas | `#060709`, one persistent full-window fill under splash, connection choice and shell (`theme::BACKGROUND`). Never a media rectangle. |
| Scene | One square-ish envelope, **144 x 176 scene units**, scaled as a unit (`orbit::ENVELOPE`). |
| Layer order, back to front | (1) rear prisms (negative depth), (2) crescent fill and polished rim, (3) rock fragments and flecks inside the crescent, (4) sphere: smoky base, molten lobes, inner glow, rim, one reflection arc, (5) front prisms (positive depth). |
| Satellites | Exactly five, persistent identities, phases 72 degrees apart. |
| Loop | **8 s**, seamless, no fade at either end. |
| Startup | **3 s**: scene at full opacity from the first frame to 2.65 s, crossfade to the connection choice 2.65 s to 3.00 s. |

### 2. Geometry (reference constants, reproduced in `orbit.rs`)

All lengths are in core radii `rc` (the sphere radius), then scaled. The reference render is 800 x 450 with `rc = 99.76 px`.

| Constant | Value |
|---|---|
| Ring radius `R` | `1.3031175 * rc` (130 px at width 800) |
| Inclination | 66 degrees |
| Lift (ring raised above the sphere center) | `38 / 99.76 * rc` (38 px at width 800) |
| Prism scale | 0.70 |
| Phase spacing | 72 degrees (`2*pi/5`) |
| Perspective | `1 / (1 - z / (7.5 * rc))` |
| Pace | one revolution per 8 s, `phase = 2*pi * (t / 8)` |

For satellite `i` at time `t`:

```text
theta = pi/2 + phase + i * 2*pi/5
x3 = R cos(theta)
y3 = R sin(theta) cos(66 deg)
z  = R sin(theta) sin(66 deg)          // z >= 0 is in front of the sphere
p  = 1 / (1 - z / (7.5 rc))
screen_x = cx + x3 * p
screen_y = cy - lift - y3 * p           // screen y grows downward
scale    = p * 0.70
yaw      = 0.82 + 0.18 * |sin(theta)|   // compresses the prism's local x
turn     = atan2(screen_y - (cy - lift), x3 * p) + pi/2 + 0.055 * sin(2*phase + i)
```

The prism is a triangle `apex (0, -2h/3)`, `left (-w/2*yaw, h/3)`, `right (w/2*yaw, h/3)` with `w = 22.7`, `h = 22.0` scene units, rotated by `turn`, scaled by `scale` and placed at the screen point. Rear prisms draw before the sphere and front prisms after it, so the sphere hides a rear prism naturally. The crescent is never drawn over a prism and nothing is clipped to avoid contact: clearance comes only from the geometry.

Crescent: outer disc centre `(72, 105)` radius 62 minus a cut disc centre `(72, 69)` radius 50, in scene units, as one polygon (`orbit::crescent`). Sphere centre `(72, 61)`, radius 40.

### 3. Mapping 8 s to 3 s

The native splash samples the same loop from `t = 0`. It does not speed up: after 3 s the phase is `2*pi * 3/8` (135 degrees), under half a turn. The crossfade starts at 2.65 s: the connection choice fades in (inert, so it takes no input) while the scene fades out, then the choice takes over at 3.00 s. A late wake after 3 s goes straight to the choice and never replays. The clock starts at the first poll after launch (`Startup::begin`), so a slow first paint cannot extend the 3 s.

### 4. Scale at compact width, fractional scale and high DPI

`orbit::display_size(window)` implements the owner's rule. With window `W x H` in logical pixels:

```text
P = clamp(0.04 * min(W, H), 16, 32)
target_height = min(0.66 * (H - 2P), 440)
scale = min((W - 2P) / 144, target_height / 176)
size  = (144 * scale, 176 * scale)
```

The canvas uses `Shape::view_box(144, 176)`, so every coordinate, including the satellites and the crescent, scales together. Stroke widths are in points and do not scale; they are hairlines (0.6 to 1.0). Device scale (1x, 2x, fractional) is the renderer's; the element is laid out in logical pixels. The view reads its previous-frame size through `cx.layout_size("startup")`, so resizing changes the scale and never the phase.

### 5. Reduced-motion static fallback

With Reduced Motion (`ViewPreferences::reduced_motion`), `startup::phase_at` never returns a crossfade. The scene is drawn at one fixed pose (`orbit::STILL_SECONDS = 1.5`), the molten lobes and prisms do not move, and the choice appears at 3.00 s. The scene is vector, so there is no asset that can fail to load; if a raster pack is added later, a failed decode must fall back to this same still, and a failed still goes straight to the choice.

### 6. Check clearance and the other invariants

Run the unit tests for the module (no GPU, no network):

```bash
python3 scripts/agent_cargo.py test --locked -p fleetworks-desktop --lib orbit
python3 scripts/agent_cargo.py test --locked -p fleetworks-desktop --lib startup
```

`orbit::tests::no_prism_edge_comes_near_the_crescent_at_any_phase` samples two full loops at 60 Hz, eight points along every edge of every prism, and measures the distance to the crescent polygon. **Pass means the minimum is at least `MIN_CLEARANCE` (5.3 units, the reference scene's own margin at this scale).** The geometry above measures about 8 units. Other tests prove five prisms persist with both front and rear passes, the whole orbit stays inside the envelope, the loop is seamless at 8 s, the first 3 s cover less than half a turn, molten lobes stay inside the sphere, and `display_size` contains the envelope at 640 x 480, 1600 x 900, 320 x 568 and 1180 x 780.

If you change a constant, re-run these before anything else. If a prism comes within the margin, change the ring (radius, inclination or lift), never the draw order.

### 7. Reference media and independent verification (optional)

The reference GIF, MP4 and poster were rendered offline from the approved baseline PNG (`assets/brand/fleetworks-material-logo.png`, 1672 x 941, SHA-256 `1ae55378...d70112`). To reproduce them you need Node.js 20 with `sharp` 0.35.5, Python 3, and FFmpeg with `ffprobe` and `libx264`. The renderer is a per-frame compositor: it blanks the original crown and sphere in the baseline, then for each of 160 frames draws the five prism sprites (cut from the baseline at the polygons below), depth-sorted around the sphere, with the sphere's interior textures flowing under a fixed glass contour. The original scripts (`animate.cjs`, `export_previews.py`, `clearance/verify_clearance.py`, `geometry.py`) are not stored in this repository. Request them from the design owner, or rebuild them from this page; the sprite polygons at 1672 x 941 are:

| Sprite | Polygon (apex first for 1 to 3; tip index 0, and 1 for sprite 5) |
|---|---|
| 1 | (838,58) (895,179) (781,179) |
| 2 | (626,126) (738,190) (649,252) |
| 3 | (1051,127) (1026,252) (937,190) |
| 4 | (482,294) (621,287) (576,391) |
| 5 | (1051,287) (1190,294) (1095,392) |

The sphere is the ellipse centred `(834.5, 435.5)` with radius 208.5. Frames render with `node animate.cjs --width 800 --frames 160 --out frames/quiet-orbit`.

Export the review media from a frame directory of numbered PNGs (20 fps, 800 wide, even dimensions, no upscaling):

```bash
ffmpeg -framerate 20 -start_number 0 -i frame-%06d.png \
  -vf "fps=20,scale=w='min(960,iw)':h=-2:flags=lanczos,format=yuv420p" \
  -an -c:v libx264 -preset medium -crf 18 -movflags +faststart quiet-orbit.mp4
ffmpeg -framerate 20 -start_number 0 -i frame-%06d.png \
  -vf "fps=20,scale=w='min(800,iw)':h=-2:flags=lanczos,palettegen=max_colors=256:reserve_transparent=1:stats_mode=full" \
  -frames:v 1 -update 1 palette.png
ffmpeg -framerate 20 -start_number 0 -i frame-%06d.png -i palette.png -filter_complex \
  "[0:v]fps=20,scale=w='min(800,iw)':h=-2:flags=lanczos[v];[v][1:v]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
  -loop 0 -gifflags +transdiff quiet-orbit.gif
```

Expected output: 160 frames, 8.0 s, GIF about 2.7 MiB (under a 15 MiB ceiling), poster at 62 percent of the duration (4.96 s). The independent clearance check (Python 3 with NumPy >= 1.24, SciPy >= 1.10, Pillow >= 9) sweeps 1,440 phases at 0.25 degrees for all five prisms, checks every covered pixel centre against the painted crescent in the rendered PNG at brightness thresholds 8 and 4 of 255, and passes when **overlap is zero**. The recorded result is minimum polygon clearance 13.3 px, painted-mask clearance 9.2 px (about 8 px after a 0.6 px sampling reserve), at 800 x 450. This is a dense sweep, not a continuous-time proof.

### 8. What to regenerate after a change

Update `assets/brand/brand-manifest.json` (geometry, timing), the GIF only if the reference scene changes, and the captures listed in the [verification checklist](../../../wiki/code/native-verification.md#splash-theme-and-live-state-review).
