# Logo, Quiet Orbit and canvas continuity

[Design index](README.md) · [Theme](theme.md) · [Asset inventory](../../../assets/brand/README.md)

## Source and appearance

Use the approved [material source](../../../assets/brand/source/fleetworks-liquid-metal-glass.png), SHA-256 `1ae553782431da7649a1d88accdacc9076c3a039f1c2b5c0018f5998d7d70112`. Its smoky glass sphere contains a molten ivory/amber core; the crescent contains coarse fractured obsidian. The runtime and exported mark consist of the warm core and crescent. Retain the sparse faint stars within the crescent. Warm metallic highlights belong to this symbol, never to ordinary controls or reading surfaces.

Brand display areas contain the symbol alone. Preserve the product name in accessibility labels, window titles, package metadata and documentation. Show functional headings, including the workstation folder picker and setup stages, below the mark.

All materials are visible in the first frame. The [static SVG](../../../assets/brand/fleetworks-mark.svg) defines the settled crescent and core. The [prior GIF](../../../assets/brand/reference/fleetworks-quiet-orbit.gif), [MP4](../../../assets/brand/reference/fleetworks-quiet-orbit.mp4), [poster](../../../assets/brand/reference/fleetworks-quiet-orbit.png) and [playback manifest](../../../assets/brand/reference/quiet-orbit/playback.json) document the earlier satellite study and underlying material treatment.

## Timing and placement

| Item | Contract |
| --- | --- |
| Native folder picker | Start with the complete orbital mark and fade the folder tiles into view in staggered 440 ms entrances. Play the core motion for 3 seconds, then hold the final pose. |
| Setup | Selecting a folder starts Embedded while the opening finishes. Move the completed orbit pose into the setup header over about 220 ms and keep the mark there through connection, registration, health checks, failure and retry. Hold each resolved checklist for two seconds. Back restores focus to the selected tile and returns the settled mark without replay. |
| Data setup | Center the active stage and controls below the settled mark. Progress dots mark connection, registration, Embedded health checks, actor catalog, actor index, optional inference, and final workstation checks. Keep the same content envelope through confirmation, progress, and outcome. |
| Enter workstation | Mount Workstation Status after setup. Move the mark into the wide or compact sidebar rail over about 200 ms and settle into the static crescent and core. |
| Scope | One startup presentation; no replay on navigation, dialog close, refresh or in-app reconnect |
| Reduced motion / asset failure | Show the complete static mark directly at each destination. Reduced Motion skips the opening delay and keeps the two-second checklist hold. |
| After startup | Ordinary pages are static apart from useful local activity; no full-canvas decorative motion loop |

Measure the complete scene envelope before sizing. Use a centered contain transform with uniform scale: `s = min(availableWidth / envelopeWidth, availableHeight / envelopeHeight)`. Calculate available dimensions after page insets, keep the envelope inside the measured native window, and convert logical points to physical pixels only at rasterization. Do not size by the sphere alone or stretch an 800×450 rectangle. Recheck at minimum 640×480, compact 800×1100, baseline 1180×780 and wide 1440×900, with 1×/2× scale and enlarged text. Logo scaling must not push the setup form outside the available viewport.

Use the same opaque `#060709` root clear color for the native window, folder picker, setup, content shell and web page. The artwork sits above the heading and centered folder grid. Selecting a folder docks the mark in the center while setup stages slide beneath it. Scene transparency uses object coverage with premultiplied alpha. Black mineral interiors remain opaque. Derive supported RGBA layers or a bounded native scene for runtime. Place exterior stars in a separate persistent canvas layer and settle their opacity to zero by service content. Missing textures use the complete static mark.

## Earlier satellite study

The [clearance report](../../../assets/brand/reference/quiet-orbit/orbit-clearance.json), [motion validation](../../../assets/brand/reference/quiet-orbit/motion-validation.json) and [orbital trace](../../../assets/brand/reference/quiet-orbit/orbital-trace.json) preserve design evidence for the earlier five-satellite composition.

## Native materials

The native scene in `orbit.rs` draws the faceted crescent, the drifting photographic core, and the polished rim. The core texture comes from the material source image.

| Part | Reference | Native render |
| --- | --- | --- |
| Core | Continuous molten flow through smoky glass; ivory and amber swirls, dark smoke at the rim | The approved sphere itself: a 384 px crop of `fleetworks-liquid-metal-glass.png` (`assets/brand/orbit/`, derived by `derive_core_texture.py`, no installs), drawn in a circle as two copies that drift against each other by whole turns of the sixteen-second loop (3.5% of the diameter at most, so the source's corners never show). Reduced motion holds the 1.5 s pose |
| Crescent | Coarse fractured obsidian with faint flecks and a polished edge | About eighty angular facets cut from a jittered grid with the shared `rock::noise` hash, tone 0 to 1 per facet, a lit edge on a third of them, warm and cool flecks, a polished outline |
## Reproduce the earlier satellite study

The preserved renderer and exporter are authoring tools under `assets/brand/reference/quiet-orbit/source/`; they are not app dependencies. From the repository root, with Node.js, Python 3 and FFmpeg available:

```sh
npm install --prefix assets/brand/reference/quiet-orbit/source
node assets/brand/reference/quiet-orbit/source/animate.cjs --source assets/brand/source/fleetworks-liquid-metal-glass.png --width 800 --frames 160 --out .agent-scratch/quiet-orbit/frames
python3 assets/brand/reference/quiet-orbit/source/export_previews.py export .agent-scratch/quiet-orbit/frames --name fleetworks-quiet-orbit --output .agent-scratch/quiet-orbit/rendered --input-fps 20 --fps 20
python3 -m pip install -r assets/brand/reference/quiet-orbit/source/clearance/requirements.txt
python3 assets/brand/reference/quiet-orbit/source/clearance/verify_clearance.py --reference-frame assets/brand/reference/fleetworks-quiet-orbit.png --output .agent-scratch/quiet-orbit/clearance.json
```

The [verifier notes](../../../assets/brand/reference/quiet-orbit/source/clearance/README.md) explain threshold, alpha-fringe and prism-depth assumptions. Keep generated frames, dependency directories and new reports in ignored scratch. Preserve source reference bytes and record deliberate derivatives separately. Production packaging must use formats supported by the native resource pipeline, bounded decoded memory and an asset-failure fallback; a GIF in documentation does not establish native GIF support. Web assets retain exact URL allowlisting and configured base-path handling.

## In-app transitions

The connected shell keeps its navigation and canvas fixed during a keyed 160 ms content fade. Brain details open from the selected result with a four-point shift. Jobs & Logs slides the selected job’s logs in from the right and brings the list back from the left. The Inbox popover shifts four points down from the toolbar bell over a dimmed workspace. Assistant and Commands enter with short four- to six-point slides and fades. Refreshes keep the same view identity, scroll and selection. With reduced motion the views swap directly and keep focus and status cues.

## Acceptance

Capture the first-frame chooser, both setup paths and the connected shell at 640×480 and a normal desktop size. Inspect the crescent and core in the splash, setup handoff and final rail at both sizes. Check for clipping, unreadable labels and canvas flashes. Operate both tiles by keyboard, verify Back restores focus without replay, and check that only Start or Connect dispatches a backend action. Repeat with reduced motion. Exercise one page change, a Brain disclosure and an overlay with both motion settings. Record installed-app results separately from production-view fixtures.
