# Brand and motion assets

The approved visual source is [source/fleetworks-liquid-metal-glass.png](source/fleetworks-liquid-metal-glass.png): molten glass core, fractured obsidian crescent and five triangular satellites. The selected animation is [reference/fleetworks-quiet-orbit.gif](reference/fleetworks-quiet-orbit.gif), with [MP4](reference/fleetworks-quiet-orbit.mp4) and [poster](reference/fleetworks-quiet-orbit.png) companions. The +30% orbital triangle elevation (`49.4 px` upward offset at 800 px width) applies to animations and videos (and their derived still/proof frames) only. The static mark uses a fixed crown.

Follow the [matte canvas design](../../docs/design/matte-canvas/README.md) and [motion reproduction guide](../../docs/design/matte-canvas/motion.md). The [reference manifest](../../docs/design/matte-canvas/reference-manifest.json) records exact source and mockup hashes. `reference/quiet-orbit/` preserves the offline renderer, export helper and prior finite clearance evidence.

The [fixed-crown mark](fleetworks-mark.svg) uses the native scene’s 144 × 176 envelope and shares the glass core, crescent and five satellite positions with the settled rail. Web uses the transparent mark and the launcher icons use the same silhouette on a matte square. Run `sh assets/brand/source/export_macos_launcher.sh` on macOS to regenerate the bundled ICNS from the Linux launcher SVG. The older `fleetworks-logo.svg` and `fleetworks-logo.jpg` remain historical assets.

Brand display areas use the symbol without a visible wordmark. Product identity remains available in accessible names, native/window metadata and documentation. Only the logo has a permanent metallic finish. Coarse matte crescent texture is reserved for Submit/Confirm/Create/Delete action faces; ordinary controls and content remain smooth matte. Runtime alpha, sizing, platform icon derivatives and packaging remain implementation work under the handoff.

The action-button rock tile lives in `rock/` (derived from the approved source by `source/derive_rock_tile.py`; PNG for web, raw RGBA for native, and a manifest with the source hash, crop and output hashes). See the [recipe](../../docs/design/matte-canvas/theme.md#the-committed-crescent-tile-implemented).

The Quiet Orbit core texture lives in `orbit/` (the approved sphere cropped from the source by `source/derive_core_texture.py`, as PNG and raw RGBA with a manifest). Verify it with `python3 assets/brand/source/derive_core_texture.py --check`.
