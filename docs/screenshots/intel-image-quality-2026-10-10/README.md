# Intel image quality review — 2026-10-10

Actual TestBackend output. All twelve retained PNGs reviewed in two six-size contact sheets; 160×50 bulletin/brief and 120×40 bulletin also inspected at full resolution.

Visual: wide bulletin image/text centered vertically; briefing image centered horizontally; aspect ratio retained. Compact bulletin retains stacked imagery and scrollable preview; 60×18 collapses imagery when too short. Below-minimum 40×20 remains constrained and needs scrolling. Image padding can consume a partial cell at odd fitted dimensions. Existing map missing-glyph boxes depend on capture font.

Interaction/source checks: source-pixel preservation, aspect/centering, all direct-terminal hints, query precedence, multiplexer guards, native Kitty/iTerm/Sixel encoders, failed/missing collapse and stale encoding pass. Browser actions and metric cohorts are unchanged; existing Intel and metric regression tests pass.

Gate: fmt, strict workspace Clippy, workspace tests with locked dependencies/no default features and ARGOS_EMBED unset. 182 binary/TUI + 709 core + 3 integration passed; 8 ignored. Initial sandbox run had two mock-socket permission failures; final suite passed with socket permissions.

Limits: synthetic fallback images verify composition, not live native graphics or provider source-image quality. No live terminal/browser/provider/default-LanceDB/MiniLM checks. User-provided screenshots supplied the before reference.

| Capture | PNG |
| --- | --- |
| intel-brief-image-100x32 | [View](intel-brief-image-100x32.png) |
| intel-brief-image-120x40 | [View](intel-brief-image-120x40.png) |
| intel-brief-image-160x50 | [View](intel-brief-image-160x50.png) |
| intel-brief-image-40x20 | [View](intel-brief-image-40x20.png) |
| intel-brief-image-60x18 | [View](intel-brief-image-60x18.png) |
| intel-brief-image-80x24 | [View](intel-brief-image-80x24.png) |
| intel-image-100x32 | [View](intel-image-100x32.png) |
| intel-image-120x40 | [View](intel-image-120x40.png) |
| intel-image-160x50 | [View](intel-image-160x50.png) |
| intel-image-40x20 | [View](intel-image-40x20.png) |
| intel-image-60x18 | [View](intel-image-60x18.png) |
| intel-image-80x24 | [View](intel-image-80x24.png) |

Provenance: [manifest.json](manifest.json). Original full capture contained 162 fixtures; only the twelve affected Intel captures are retained here.
