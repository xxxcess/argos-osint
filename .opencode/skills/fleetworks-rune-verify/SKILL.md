---
name: fleetworks-rune-verify
description: Design, implement, and review Fleetworks Desktop on Linux x86-64 and macOS using Rune, Glass Canvas, accessible interactions, and inspected production-view fixtures. Use for native screens, components, motion, and interaction states. Intel Web uses its own React workflow.
---

# Fleetworks native design and Rune verification

Read root/scoped `AGENTS.md`, [argos-testing](../argos-testing/SKILL.md), and [native verification](../../../docs/wiki/code/native-verification.md); reuse authorized scope and one integration/build owner. When changing the harness, inspect the pinned Rune/kit README (Agent tooling & Accessibility), hello example, and tooling args at the exact `Cargo.toml`/`Cargo.lock` revision.

## Design the interaction

Read [designing a native interaction](../../../docs/wiki/code/native-design-system.md#designing-a-native-interaction) before choosing controls. Work from operator task, data/authority, and next action; keep connection status, selection, and next action visible at minimum width.

- **Composition:** Apply [Glass Canvas](../../../docs/design/matte-canvas/theme.md) via `theme.rs`, `material.rs`, and `ui_components.rs` (`GlassSurface` + crisp static liquid-metal rims, deterministic `ViewPreferences::matte()` opaque fallback, `#060709` backdrop, warm molten-gold/cyan accents, fractured crescent-rock texture on committing/brand actions). Follow the calm desktop rhythm (slim smoked-glass left rail, empty toolbar title area, compact top-right telemetry/bell strip, hairline-grouped glass settings, floating frosted-glass Inbox popover). Lists scan, tables compare, cards group shared decisions; pair status color with label/icon.
- **Input, motion & states:** Keep press/selection/focus distinct, keyboard order, fixed hit bounds, Escape, and focus return. Follow [motion](../../../docs/design/matte-canvas/motion.md) (Quiet Orbit & reduced-motion) and [interaction-state](../../../docs/design/matte-canvas/interaction-state.md): design unresolved/settled/refreshing/unavailable/failed/success states together, preserve resolved text/identity/selection/scroll during refresh, localize progress/retry, and keep explicit mutation confirmation.

## Verify the production view

Production and fixtures share `app::view`, `State`, theme, and handlers; [native_fixture](../../../apps/fleetworks-desktop/examples/native_fixture.rs) injects synthetic effects (never real user data). Assert observable state/input with Rune Runtime before capturing images, adding finite regressions for affected stale replies, permissions, pagination/follow, scope changes, errors, keyboard/focus, and protected paths.

- **Runtime tips:** `Runtime::find` returns drawn centre without checking clipping—for offscreen controls call `Tx::scroll_into_view`, advance a frame, and assert centre lies in the scroll viewport before pointer input. After `Runtime::window_focus`, advance a frame before asserting handler effects.
- **CI vs. local:** In Nightly GSD, build, test, and capture via [GitHub Actions](../../../.agents/skills/gsd/SKILL.md#github-ci-first) and use the artifact for live review. For authorized local builds, reuse `target/agents/debug` via `python3 scripts/agent_cargo.py … --locked`; view-only edits need neither Linux engines, Node/Docker, nor the embedded matrix.

Generate real-view captures with the build-once helper:

```sh
python3 scripts/fleetworks_verify.py native-ui --agent \
  --scene brain-search-ready --size 1280x800 --scale 2 \
  --output .agent-scratch/native-ui/selected-review
```

Pick scenes from actual executable inventory (zero/unknown/missing/stale outputs fail), covering compact/wide, scale, loading/error, `matte()`/material, a11y, and bounded input scripts/final tree keys where interaction matters. Never substitute terminal cells or concept mockups for Runtime evidence.

Open and inspect emitted PNGs against `docs/design/matte-canvas/` (symbol-only crescent + molten-core branding with no orbit triangles, layered glass + static rims + `matte()` fallback, crescent-rock on Submit/Confirm/Create/Delete/Connect and brand accents, local loading sheen, stable continuous scroll, legible text/focus/contrast/clipping). Record generated vs. inspected separately; headless captures do not prove Launch Services, real dialogs/clipboard, system prefs, minimum OS, signing, live inference, or hardware GUI behavior.

For PR review, use pass-one [Linux candidate + PNG/tree artifacts](../../../docs/wiki/code/ci.md#nightly-gsd-order); once green, dispatch macOS/Web checks and [dispatchable visual review](../../../docs/wiki/code/native-verification.md#dispatchable-ui-visual-review) at the same SHA. Keep synthetic captures, physical app behavior, and transport evidence distinct.
