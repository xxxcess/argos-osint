# Fleetworks brand assets

| File | Role |
|---|---|
| `fleetworks-material-logo.png` | Rich logo and README hero. Byte-identical canonical source. |
| `fleetworks-quiet-orbit.gif` | Quiet Orbit motion reference, 800 x 450, 160 frames at 20 fps (8 s). Review media; the native app does not ship it. |
| `fleetworks-logo.svg`, `fleetworks-mark.svg` | Small-size vector family for headers, icons and favicons. |
| `fleetworks-logo.jpg` | Historical banner. Not used by the app. |
| `brand-manifest.json` | Hashes, geometry and timing for every file above. |

The native splash is drawn from vector geometry in `apps/fleetworks-desktop/src/orbit.rs`, over the
shared `#060709` canvas. `brand-manifest.json` records its geometry and timing.
