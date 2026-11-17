# Fleetworks — selected Quiet Orbit

## Selected direction

The user selected **Quiet Orbit** and requested that the satellite triangles stay clear of the moon crescent. Quiet Orbit is the production direction; its corrected orbit geometry is preserved in this revision. The expanded handoff adds repository asset adoption, seamless native canvas composition, service-theme continuity, responsive grids and stable realtime loading/state presentation.

The material reference remains `fleetworks-liquid-metal-glass-baseline.png`, SHA-256 `1ae553782431da7649a1d88accdacc9076c3a039f1c2b5c0018f5998d7d70112`: a molten ivory/amber interior inside a smoky glass sphere, and coarse fractured obsidian inside a polished crescent shell.

All materials are visible from the first frame. Five opaque glass triangular satellites remain in continuous orbit. The sphere may naturally hide a rear satellite; the crescent must remain visibly separate from every triangle.

## Crescent clearance contract

Check the entire transformed triangle silhouette, including perspective, orientation, bevel and antialias coverage. A centerpoint-only check is insufficient. Clearance applies before any planet occlusion, so hidden portions cannot pass through the crescent unnoticed.

Use one coherent fixed orbital path with a visible safety margin. Preserve five persistent satellite identities and smooth periodic motion. Cropping, hiding, per-frame position corrections, or drawing the crescent over an intersecting triangle do not satisfy this requirement.

The selected geometry at 800 pixels wide is a **130-pixel orbital radius, 66° inclination, 38-pixel upward projection offset and 0.70 satellite scale**. Five phases stay evenly spaced by 72°. Scale these values proportionally with the scene; use the same continuous path throughout the loop.

`orbit-clearance.json` records the dense phase sweep and minimum separation. `fleetworks-clearance-proof.png` shows the closest geometric pass at 114.5°. `fleetworks-clearance-raster-proof.png` shows the closest pass to the painted crescent fringe at 244.25°. Verify the actual painted crescent boundary as well as the ideal curve, and retain a conservative margin for bevel, antialias coverage and interpolation.

## Verified preview clearance

The independent sweep checked **1,440 phases and all five satellites**: 7,200 complete triangle poses and 6,022,018 covered-pixel samples, including the alpha fringe. It found **zero overlap with the painted crescent**. At 800×450, conservative pixel-edge separation is 9.216 pixels from the visible rim and 8.435 pixels from its very faint fringe; allowing a further 0.6 pixel for phase sampling leaves approximately **8 pixels** of clearance. This is a dense finite sweep with an engineering margin, not a formal continuous-time proof.

All five objects retain a complete winding around the actual core and both front/rear passes. The renderer has no crescent mask or crescent hiding pass. The report also documents the explicit prism-thickness assumptions used for the 2.5D core-volume check. Reproduce the checks using `source/clearance/README.md`; implementation must repeat them for native sizes and transforms.

## Production integration and continuity

The canonical PNG/GIF remain byte-identical references. `repository-adoption.json` supplies their repository destination paths. The GIF's baked black background is retained for review; the production app derives proper RGBA object coverage over the shared `#060709` canvas, so no image rectangle appears. Exterior space/stars belong to the persistent viewport background. Do not remove opaque black obsidian pixels using brightness-based keying.

Read `fleetworks-splash-handoff.md` first, then both mandatory companions: `fleetworks-deep-space-theme.md` and `fleetworks-realtime-state-contract.md`. They require proportional full-orbit containment and DPI handling, consistent material quality through splash/gateway/services, semantic colors, measured responsive grids, and stable loading/live content with restrained skeleton shimmer and concise labels. This package contains design/source evidence, not a native implementation capture.

## Files and timing

- `fleetworks-01-quiet-orbit.gif` — selected animated preview.
- `fleetworks-01-quiet-orbit.mp4` — smoother tonal reproduction for detailed review.
- `fleetworks-01-quiet-orbit.png` — poster.
- `fleetworks-splash-handoff.md` — implementation contract and verified edit map.
- `orbit-clearance.json`, `motion-validation.json` and `orbital-trace.json` — geometry, rendered-loop checks and five-object motion evidence.
- `fleetworks-clearance-proof.png` and `fleetworks-clearance-raster-proof.png` — closest-pass stills.
- `source/` — reusable offline renderer, export helper and clearance verifier.

The review loop remains **eight seconds, 160 frames, 20 fps**. It has no initial shape/material reveal or ending fade.

Native startup remains **three seconds**: the complete scene animates immediately, the gateway setup appears beneath the crossfade at **2.65 seconds**, and input/focus transfer completes at **3.00 seconds**. The native animation samples the orbit at its quiet pace; it need not finish a revolution during startup.

The other four approaches remain earlier design studies. They are not competing implementation options in this selected package.

## Rendering and implementation evidence

The preview uses independent texture-backed components, geometric orbital coordinates and perspective, depth ordering, and localized molten-flow reprojection. It is a **2.5D motion study**. It does not establish physical volumetric simulation or native-app behavior.

The handoff retains the dated repository audit. Native builds, captures, installed-app checks and resource measurements remain implementation work. GIF/MP4 previews are review media; the app should use the bounded native layers described in the handoff.

## Reproduce

The full, self-contained procedure is in [the splash handoff, Reproduce](fleetworks-splash-handoff.md#reproduce): geometry and projection formulas, layer order, the 8 s to 3 s mapping and crossfade, scale rules, the reduced-motion still, the clearance test and its pass criterion, and the exact `ffmpeg` commands for the GIF, MP4 and poster. The short version:

1. **Native scene.** `apps/fleetworks-desktop/src/orbit.rs` draws the scene from the constants above. `python3 scripts/agent_cargo.py test --locked -p fleetworks-desktop --lib orbit` proves clearance (minimum 5.3 scene units, measured about 8), five persistent prisms with front and rear passes, containment in the envelope and a seamless 8 s loop.
2. **Review media.** The GIF is 800 x 450, 160 frames, 20 fps, 8.0 s, canvas baked black for review only. The native app composites over `#060709` and never ships the GIF. It is kept byte-identical at `assets/brand/fleetworks-quiet-orbit.gif` (SHA-256 `22d4e6a2...7103d57`).
3. **Re-render (optional).** Needs Node.js 20 with `sharp` 0.35.5, Python 3 and FFmpeg with `libx264`. Render frames with the reference compositor (`node animate.cjs --width 800 --frames 160 --out frames/quiet-orbit`), then export with the commands in the handoff. The scripts live with the design owner and are not in this repository; the handoff lists the polygons and constants they use.
4. **Independent clearance replica (optional).** Python 3 with NumPy >= 1.24, SciPy >= 1.10, Pillow >= 9 reads the 800 x 450 poster and sweeps 1,440 phases. Pass means zero overlap with the painted crescent. These tools are offline and add no runtime dependency to Fleetworks.

The reference loop has no opening reveal or ending fade; the native startup additionally crossfades into the connection choice at 2.65 s.
