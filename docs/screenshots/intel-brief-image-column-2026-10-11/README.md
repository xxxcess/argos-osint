# Intel brief image column — 2026-10-11

Actual TestBackend output. The article image is no longer a full-width strip above the
`brief article` section; it now occupies a left column **inside** that section, beside the
brief content, with image and prose vertically centered in the section.

Adaptive sizing (from the reader column, so it tracks the terminal size):

- width ≈ 35% of the reader inner width, 14–40 columns, always leaving ≥ 24 columns of brief prose;
- height ≈ 20% of the reader height, 5–12 rows, never taller than the column is wide in pixel terms;
- collapses to zero width when the reader is too narrow (40×20) or when the image is missing,
  invalid or failed — the prose then keeps the full width.

Vertical padding above/below the image glyph block, measured from the captured cells:

| Size | Section inner rows | Image rows | Pad above / below |
| --- | --- | --- | --- |
| 160×50 | 9 | 7 | 1 / 1 |
| 120×40 | 7 | 7 | 0 / 0 |
| 100×32 | 7 | 5 | 1 / 1 |
| 80×24 | 6 | 5 | 0 / 1 |
| 60×18 | 7 | 5 | 1 / 1 |
| 40×20 | — | — | collapsed, prose full width |

Limit: these buffers use the deterministic half-block fixture; live Kitty/Sixel/iTerm2
graphics and provider source-image quality are not exercised here.
The stacked arrangement shown by [2026-10-10](../intel-image-quality-2026-10-10/README.md)
is superseded for the brief reader; the bulletin hero view is unchanged.

| Capture | PNG |
| --- | --- |
| intel-brief-image-160x50 | [View](intel-brief-image-160x50.png) |
| intel-brief-image-120x40 | [View](intel-brief-image-120x40.png) |
| intel-brief-image-100x32 | [View](intel-brief-image-100x32.png) |
| intel-brief-image-80x24 | [View](intel-brief-image-80x24.png) |
| intel-brief-image-60x18 | [View](intel-brief-image-60x18.png) |
| intel-brief-image-40x20 | [View](intel-brief-image-40x20.png) |

Provenance: [manifest.json](manifest.json). Full capture (162 fixtures, plan
`.planning/2026-10-11-intel-brief-image-column/tui/run-01`) reviewed by the build agent;
only the six affected brief captures are retained here.
