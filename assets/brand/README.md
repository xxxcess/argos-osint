# Brand and motion assets

The [static brand image](fleetworks-mark.svg) uses the native scene’s 144 × 176 envelope. The glass core and obsidian crescent carry through to the native rail and Intel Web. The [logo](fleetworks-logo.svg) and launcher icons place the same silhouette on a matte square. Run `sh assets/brand/source/export_macos_launcher.sh` on macOS to regenerate the bundled ICNS from the Linux launcher SVG.

The [material source](source/fleetworks-liquid-metal-glass.png) supplies the core and rock textures. The [older logo JPEG](fleetworks-logo.jpg) and Quiet Orbit [GIF](reference/fleetworks-quiet-orbit.gif), [MP4](reference/fleetworks-quiet-orbit.mp4) and [poster](reference/fleetworks-quiet-orbit.png) preserve the earlier satellite study. The [reference manifest](../../docs/design/matte-canvas/reference-manifest.json) records source hashes. See the [matte canvas design](../../docs/design/matte-canvas/README.md) and [motion contract](../../docs/design/matte-canvas/motion.md) for the current presentation.

Brand display areas use the symbol without a visible wordmark. Product identity remains available in accessible names, native/window metadata and documentation. Only the logo has a permanent metallic finish. Coarse matte crescent texture is reserved for Submit/Confirm/Create/Delete action faces; ordinary controls and content remain smooth matte.

The action-button rock tile lives in `rock/` (derived from the approved source by `source/derive_rock_tile.py`; PNG for web, raw RGBA for native, and a manifest with the source hash, crop and output hashes). See the [recipe](../../docs/design/matte-canvas/theme.md#the-committed-crescent-tile-implemented).

The Quiet Orbit core texture lives in `orbit/` (the approved sphere cropped from the source by `source/derive_core_texture.py`, as PNG and raw RGBA with a manifest). Verify it with `python3 assets/brand/source/derive_core_texture.py --check`.
