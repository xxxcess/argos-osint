# Quiet Orbit, matte theme and live states

The owner's brand and live-UI package, adopted into the repository with step-by-step reproduction. Start with the splash, then the theme, then the state contract.

| Document | What it covers | Reproduce section |
| --- | --- | --- |
| [Splash handoff](fleetworks-splash-handoff.md) | Quiet Orbit scene, 3 s startup, crossfade, scale rules, reduced-motion still, edit map | [Reproduce](fleetworks-splash-handoff.md#reproduce): geometry, layer order, clearance test, re-render and export commands |
| [Motion reference](fleetworks-motion-reference.md) | Selected motion, clearance contract, review media | [Reproduce](fleetworks-motion-reference.md#reproduce) |
| [Matte deep-space theme](fleetworks-deep-space-theme.md) | Tokens, status colors, where each lives in native and web, fractured-obsidian texture, shimmer, contrast | [Sections 4 to 7](fleetworks-deep-space-theme.md#4-fractured-obsidian-texture) |
| [Realtime state contract](fleetworks-realtime-state-contract.md) | Loading, refresh, stale, pending and completion; Jobs cursor loading | [Reproduce](fleetworks-realtime-state-contract.md#reproduce): state machine and review steps |

## Decisions applied here

- The owner's PR addendum overrides the package where they differ. The theme is **matte**; the package's earlier liquid-glass controls are not used.
- The splash is **vector** (`apps/fleetworks-desktop/src/orbit.rs`). The reference PNG and GIF are kept under `assets/brand/` and not shipped. The package's source scripts are not in the repository.
- The package paths under `.planning/2026-11-16-brand-space-live-ui/` were replaced by this durable folder so the docs can be linked from the indexes.

## Related

[Native design system](../../../wiki/code/native-design-system.md) · [Cold-launch startup](../../../wiki/code/native-connections.md#cold-launch-startup) · [Native verification](../../../wiki/code/native-verification.md#splash-theme-and-live-state-review) · [React UI rules](../../../wiki/code/react-ui.md) · [Brand assets](../../../../assets/brand/README.md)
