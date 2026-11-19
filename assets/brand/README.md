# Brand and motion assets

The approved visual source is [source/fleetworks-liquid-metal-glass.png](source/fleetworks-liquid-metal-glass.png): molten glass core, fractured obsidian crescent and five triangular satellites. The selected animation is [reference/fleetworks-quiet-orbit.gif](reference/fleetworks-quiet-orbit.gif), with [MP4](reference/fleetworks-quiet-orbit.mp4) and [poster](reference/fleetworks-quiet-orbit.png) companions. The +30% orbital triangle elevation (`49.4 px` upward offset at 800 px width) applies to animations and videos (and their derived still/proof frames) only. The static crown keeps its fixed five-prism arrangement.

Follow the [matte canvas design](../../docs/design/matte-canvas/README.md) and [motion reproduction guide](../../docs/design/matte-canvas/motion.md). The [reference manifest](../../docs/design/matte-canvas/reference-manifest.json) records exact source and mockup hashes. `reference/quiet-orbit/` preserves the offline renderer, export helper and prior finite clearance evidence.

## Static identity

| Asset | Where it appears |
| --- | --- |
| [Photographic source](source/fleetworks-liquid-metal-glass.png) and [1024 px JPEG](fleetworks-logo.jpg) | Large identity artwork. All five prisms form a fixed crown over the smoky ivory core and fractured crescent. |
| [Transparent SVG mark](fleetworks-mark.svg) | The small-size rendition uses the fixed crown, a broad ivory/amber highlight across the sphere and a bright mineral edge. The shared Intel component embeds this vector for base-path independence. |
| [Matte-backed SVG](fleetworks-logo.svg) | Web icon routes carry the same mark on the canvas color. The public mark remains transparent. |
| [Linux launcher SVG](../../apps/fleetworks-desktop/resources/fleetworks.svg) and [macOS launcher ICNS](../../apps/fleetworks-desktop/resources/fleetworks.icns) | Rounded matte tile. The macOS bundle declares the ICNS in its Info.plist. |
| [Quiet Orbit](reference/fleetworks-quiet-orbit.mp4) | Startup uses depth-ordered orbital positions. The current native rail uses a still pose of the scene. |

After changing the canonical mark or photographic source, run `python3 assets/brand/source/export_static_marks.py` with Pillow and Inkscape installed. The exporter synchronizes the public vector, shared component, platform resources and JPEG. Brand regions display the symbol alone. Product identity remains available in accessible labels, window metadata and documentation.

The action-button rock tile lives in `rock/` (derived from the approved source by `source/derive_rock_tile.py`; PNG for web, raw RGBA for native, and a manifest with the source hash, crop and output hashes). See the [recipe](../../docs/design/matte-canvas/theme.md#the-committed-crescent-tile-implemented).

The Quiet Orbit core texture lives in `orbit/` (the approved sphere cropped from the source by `source/derive_core_texture.py`, as PNG and raw RGBA with a manifest). Verify it with `python3 assets/brand/source/derive_core_texture.py --check`.
