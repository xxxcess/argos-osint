# Logo, Quiet Orbit and canvas continuity

[Design index](README.md) · [Theme](theme.md) · [Asset inventory](../../../assets/brand/README.md)

## Source and appearance

Use the approved [material source](../../../assets/brand/source/fleetworks-liquid-metal-glass.png), SHA-256 `1ae553782431da7649a1d88accdacc9076c3a039f1c2b5c0018f5998d7d70112`. Its smoky glass sphere contains a molten ivory/amber core; the crescent contains coarse fractured obsidian. Five glass triangular satellites preserve the brand geometry. Retain the sparse faint stars within the crescent. Warm metallic highlights belong to this symbol, never to ordinary controls or reading surfaces.

Brand display areas contain the symbol alone. Remove the visible wordmark in the splash, navigation header and setup brand region; preserve the product name in accessibility labels, window titles, package metadata and documentation. This is not a repository, API, package or application-identity rename. Screen titles such as Setup Gateway remain functional headings outside the brand region.

All materials are visible in the first frame. There is no flat-shape, outline-only, assembly or shape-to-glass introduction. Use the selected [Quiet Orbit GIF](../../../assets/brand/reference/fleetworks-quiet-orbit.gif), SHA-256 `772a48940527a5fe04121bb7cc5aec443cf33ffbec9ec518b960ac041096c797`. The [MP4](../../../assets/brand/reference/fleetworks-quiet-orbit.mp4) (SHA-256 `d3871331a96c2a8ffcc859c10cadc63992b70fe428751d990983e463f76fa650`, 30 fps) retains smoother tonal detail; the [poster](../../../assets/brand/reference/fleetworks-quiet-orbit.png) (SHA-256 `c247cf0cc97c1beec3ad077e372b8bbb8ef032ed070e3a58eb714515369b5463`) supplies a static reference. These are 2.5D texture-backed design studies, not volumetric simulation or application recordings.

The [playback manifest](../../../assets/brand/reference/quiet-orbit/playback.json) records these design constants and source identities; it is not an implemented native runtime manifest.

## Timing and placement

| Item | Contract |
| --- | --- |
| Review loop | 800×450, 16 seconds (MP4: 480 frames at 30 fps; GIF: 240 frames at 15 fps); seamless periodic orbit and core flow |
| Native startup | 3 seconds at the quiet orbital pace; do not accelerate a full revolution into that interval |
| Transition | Setup appears beneath the scene at 2.65 seconds; opacity handoff and input/focus transfer finish by 3.00 seconds |
| Scope | One startup presentation; no replay on navigation, dialog close, refresh or in-app reconnect |
| Reduced motion / asset failure | Complete static logo; accessible setup remains available; no initial flat reveal or blank blocked screen |
| After startup | Ordinary pages are static apart from useful local activity; no full-canvas decorative motion loop |

Measure the complete scene envelope, including every satellite pose, before sizing. Use a centered contain transform with uniform scale: `s = min(availableWidth / envelopeWidth, availableHeight / envelopeHeight)`. Calculate available dimensions after page insets, keep the envelope inside the measured native window, and convert logical points to physical pixels only at rasterization. Do not size by the sphere alone or stretch an 800×450 rectangle. Recheck at minimum 640×480, compact 800×1100, baseline 1180×780 and wide 1440×900, with 1×/2× scale and enlarged text. Logo scaling must not push the setup form outside the available viewport.

Use the same opaque `#060709` root clear color for native window, splash host, setup, content shell and web page. Scene transparency uses object coverage, with premultiplied alpha consistently applied through interpolation/compositing; black mineral interiors remain opaque. Do not use brightness/chroma keying to remove exterior black. The baked reference GIF stays unchanged and is not a production background layer. Derive supported RGBA layers or a bounded native scene for runtime; any exterior space/stars belong to a separate persistent canvas layer rather than the logo rectangle. Working screens use the theme's uniform canvas; settle exterior star opacity to zero by service content, without an exposure step or image-backing boundary. Missing textures fall back to the complete static mark, not a new loading screen.

## Collision and orbital geometry

At 800 pixels scene width, use orbital radius **130 px**, inclination **66°**, upward projected offset **49.4 px** (+30% over the 38 px baseline; applies to **animations and videos only**, not static brand images such as `fleetworks-liquid-metal-glass.png` or `fleetworks-mark.svg`), satellite scale **0.70**, and five persistent identities separated by **72°**. Scale the geometry uniformly with the scene. The core reference radius is approximately 99.76 px. Keep the coherent fixed orbit and smooth front/rear depth ordering; the sphere may naturally occlude a rear prism. The crescent must remain separate from the full painted satellite silhouettes, including bevel and antialias coverage, before sphere occlusion is applied.

Do not hide a collision with a crescent mask, crop, disappearing satellite, per-frame position correction or discontinuous orbit. Preserve the crown-like arrangement during the upper pass while keeping all five satellites on the selected continuous route. A center-point-only clearance check is insufficient.

The carried [clearance report](../../../assets/brand/reference/quiet-orbit/orbit-clearance.json) records 1,440 sampled phases and 7,200 triangle poses with zero painted-crescent overlaps for the prior 38 px / 8 s 800×450 baseline. The faint-fringe pixel-edge gap at 38 px lift was 8.435 px (roughly 8 px after a 0.6 px finite-sampling reserve); raising the upward projected offset by 30% to 49.4 px moves all satellites further above the crescent (`fleetworks-clearance-proof.png` and `fleetworks-clearance-raster-proof.png` show the updated poses at 49.4 px). This is prior finite-sweep evidence with an engineering margin, not a continuous-time proof or native-runtime result. [Motion validation](../../../assets/brand/reference/quiet-orbit/motion-validation.json) and [orbital trace](../../../assets/brand/reference/quiet-orbit/orbital-trace.json) preserve the original provenance. Repeat clearance after changing mesh, alpha coverage, projection, timing interpolation or output scale.

## Native materials

The native scene is a bounded render, not the baked GIF: `orbit.rs` draws a back layer (rear prisms, crescent), the sphere's photographic body, then a front layer (rim line, front prisms). The core is a texture derived from the approved source image, the crescent and prisms are vector geometry. Compared with the reference poster, the aim is:

| Part | Reference | Native render |
| --- | --- | --- |
| Core | Continuous molten flow through smoky glass; ivory and amber swirls, dark smoke at the rim | The approved sphere itself: a 384 px crop of `fleetworks-liquid-metal-glass.png` (`assets/brand/orbit/`, derived by `derive_core_texture.py`, no installs), drawn in a circle as two copies that drift against each other by whole turns of the sixteen-second loop (3.5% of the diameter at most, so the source's corners never show). Reduced motion holds the 1.5 s pose |
| Crescent | Coarse fractured obsidian with faint flecks and a polished edge | About eighty angular facets cut from a jittered grid with the shared `rock::noise` hash, tone 0 to 1 per facet, a lit edge on a third of them, warm and cool flecks, a polished outline |
| Prisms | Dark glass triangles with bright edges and depth | Smoky body, lit and shaded facet, inner bevel, bright lit edge, faint rim and a warm reflection near the base |

Compared on the matching still pose (1.5 s, which is also the reduced-motion pose), the 2.65 s handoff pose and the loop start: the core now shows the approved molten flow and glass reflections instead of overlapping bands; the crescent still reads as angular rock with a polished edge but is not photographic; the prisms show facets and depth; the front and back passes, the 5.3 unit clearance and the three-second startup are unchanged. Remaining differences are visible and left for visual review: the core's motion is a shear of one still, not fluid; the crescent is vector facets, and the reference crescent is also shaped differently (a flatter, wider bowl whose centre sits about 15 scene units below the core, against 44 here), so a photographic crescent needs an agreed re-fit of its geometry and a rerun of the clearance check; the prism glass is flat-shaded.

Clearance after the material change: the geometry and its test are unchanged (about 8 scene units at the closest approach), and a new test subtracts 1.3 units for the painted reach (prism rim 0.3, crescent edge 0.5, one antialiasing step 0.5) and still requires 5.3. The authoring clearance scripts need numpy, scipy and Pillow and were not run, so no new raster measurement is claimed.

To compare a pose without building the app, run the preview port of the scene (an authoring aid that mirrors `orbit.rs` by hand and does not use the Rune renderer), then rasterize the SVG with any browser:

```sh
python3 assets/brand/reference/quiet-orbit/source/native_preview.py 1.5 .agent-scratch/quiet-orbit/native-1.5.svg 5
```

Use `1.5` for the still pose, `2.65` for the handoff and `0` for the loop start. The real check remains the native fixture captures in [native verification](../../wiki/code/native-verification.md).

## Offline reproduction

The preserved renderer and exporter are authoring tools under `assets/brand/reference/quiet-orbit/source/`; they are not app dependencies. From the repository root, with Node.js, Python 3 and FFmpeg available:

```sh
npm install --prefix assets/brand/reference/quiet-orbit/source
node assets/brand/reference/quiet-orbit/source/animate.cjs --source assets/brand/source/fleetworks-liquid-metal-glass.png --width 800 --frames 160 --out .agent-scratch/quiet-orbit/frames
python3 assets/brand/reference/quiet-orbit/source/export_previews.py export .agent-scratch/quiet-orbit/frames --name fleetworks-quiet-orbit --output .agent-scratch/quiet-orbit/rendered --input-fps 20 --fps 20
python3 -m pip install -r assets/brand/reference/quiet-orbit/source/clearance/requirements.txt
python3 assets/brand/reference/quiet-orbit/source/clearance/verify_clearance.py --reference-frame assets/brand/reference/fleetworks-quiet-orbit.png --output .agent-scratch/quiet-orbit/clearance.json
```

The [verifier notes](../../../assets/brand/reference/quiet-orbit/source/clearance/README.md) explain threshold, alpha-fringe and prism-depth assumptions. Keep generated frames, dependency directories and new reports in ignored scratch. Preserve source reference bytes and record deliberate derivatives separately. Production packaging must use formats supported by the native resource pipeline, bounded decoded memory and an asset-failure fallback; a GIF in documentation does not establish native GIF support. Web assets retain exact URL allowlisting and configured base-path handling.

## Acceptance

Compare actual splash → setup → services captures on one canvas. Inspect the full orbit, transition midpoint and final setup frame for clipped satellites, moon intersection, background rectangles, black/gray flashes and wordmarks. Verify focus transfers once, setup can proceed offline, reduced-motion behavior is static, and hidden/unfocused windows stop unnecessary animation. Record actual decode time, startup impact, frame pacing and decoded memory against the current app baseline before adopting runtime derivatives. The documentation change only imports the selected references and the existing authoring evidence.
