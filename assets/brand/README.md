# Brand and motion assets

The approved visual source is [source/fleetworks-liquid-metal-glass.png](source/fleetworks-liquid-metal-glass.png): molten glass core, fractured obsidian crescent and five triangular satellites. The selected animation is [reference/fleetworks-quiet-orbit.gif](reference/fleetworks-quiet-orbit.gif), with [MP4](reference/fleetworks-quiet-orbit.mp4) and [poster](reference/fleetworks-quiet-orbit.png) companions.

Follow the [matte canvas design](../../docs/design/matte-canvas/README.md) and [motion reproduction guide](../../docs/design/matte-canvas/motion.md). The [reference manifest](../../docs/design/matte-canvas/reference-manifest.json) records exact source and mockup hashes. `reference/quiet-orbit/` preserves the offline renderer, export helper and prior finite clearance evidence.

The older `fleetworks-mark.svg`, `fleetworks-logo.svg` and `fleetworks-logo.jpg` remain legacy/fallback assets while runtime consumers are migrated. They are not the authority for new UI accent colors. Do not delete a referenced legacy asset or claim a resource consumer is migrated merely because the source PNG/GIF was added.

Brand display areas use the symbol without a visible wordmark. Product identity remains available in accessible names, native/window metadata and documentation. Only the logo has a permanent metallic finish. Coarse matte crescent texture is reserved for Submit/Confirm/Create/Delete action faces; ordinary controls and content remain smooth matte. Runtime alpha, sizing, platform icon derivatives and packaging remain implementation work under the handoff.

The action-button rock tile lives in `rock/` (derived from the approved source by `source/derive_rock_tile.py`; PNG for web, raw RGBA for native, and a manifest with the source hash, crop and output hashes). See the [recipe](../../docs/design/matte-canvas/theme.md#the-committed-crescent-tile-implemented).
